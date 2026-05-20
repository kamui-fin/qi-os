pub mod devfs;
pub mod pipe;
pub mod sys;

use core::any::Any;
use core::ffi::CStr;
use core::sync::atomic::AtomicU64;

use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::ffi::c_str;
use alloc::string::{String, ToString};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use alloc::{format, vec};
use bitfield_struct::bitfield;
use bitflags;
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use elf::segment;
use lazy_static;
use spin::{Spinlock, RwLock};

use crate::driver::ata::{AtaDriver, BusType, DriveType};
use crate::driver::cmos::get_unix_time;
use crate::fs::fat::{BlockDevice, DirEntryWithLoc, FSInfo, Fat32, BPB};
use crate::fs::ustar::USTAR;
use crate::fs::vfs::devfs::mount_devfs;
use crate::fs::vfs::pipe::Pipe;
use crate::task::proc::curr_proc;
use crate::task::thread::{block_task, block_task_drop_lock, BlockReason, ThreadState, SCHEDULER};
use crate::{serial_println, PROC};

pub static MOUNT_TABLE: OnceCell<MountTable> = OnceCell::uninit();

pub struct Mount {
    pub root: Arc<RwLock<DEntry>>,
    pub mountpoint: Arc<RwLock<DEntry>>,
    pub sb: Arc<SuperBlock>,
}

pub type MountTable = alloc::collections::BTreeMap<String, Mount>;

pub enum FsType {
    Fat32,
    Ustar,
    PipeFs,
    DevFs,
}

pub struct SuperBlock {
    pub fs_type: FsType,
}

#[derive(Debug, Clone, Copy)]
pub enum NodeType {
    File,
    Directory,
    Pipe,
    CharDevice,
}

pub enum INodeData {
    Pipe(Arc<Pipe>),
    Device { major: u8, minor: u8 },
    FatNode(Spinlock<DirEntryWithLoc>),
    DevFs(Spinlock<BTreeMap<String, Arc<INode>>>),
}

pub struct INode {
    pub inum: u64,
    pub fs: Arc<SuperBlock>,

    // these two MUST stay in sync
    pub mode: NodeType,
    pub data: INodeData,

    pub meta: Spinlock<FsMetadata>,

    pub ops: Arc<dyn INodeOps>,
}

pub struct FsMetadata {
    pub size: usize,
    pub mtime: u64, // content OR metadata changes

    pub dirty: bool,
}

pub trait FileOps: Send + Sync {
    fn read(&self, file: &File, buffer: &mut [u8]) -> usize;
    fn write(&self, file: &File, buffer: &[u8]) -> usize;
    fn close(&self, file: &File) {}
    fn ioctl(&self, cmd: u64, arg: u64) {}
}

pub trait INodeOps: Send + Sync {
    fn open(&self, inode: &INode, flags: OpenFlags) -> Arc<dyn FileOps>;
    fn lookup(&self, parent: &INode, name: &str) -> Option<Arc<INode>> {
        None
    }
    fn readdir(&self, dir: &INode) -> Vec<DEntryMinimal> {
        vec![]
    }
    fn create_file(&self, dir: &INode, name: &str) {}
    fn rename(&self, node: &INode, to: &str) {}
    fn mkdir(&self, dir: &INode, name: &str) {}
    fn delete_file(&self, file: &INode) {}
    fn rmdir(&self, dir: &INode) {}
    fn stat(&self, node: &INode) -> Stat {
        Stat::default()
    }
}

pub struct DEntry {
    pub name: String,
    pub inode: Arc<INode>,
    pub parent: Option<Weak<RwLock<DEntry>>>, // None if root
    // cache of children we alr looked up
    pub children: RwLock<BTreeMap<String, Arc<RwLock<DEntry>>>>,
}

#[derive(Clone)]
pub struct DEntryMinimal {
    pub name: [u8; 64],
    pub inum: u64,
    pub filetype: NodeType,
    pub size: usize,
}

#[derive(Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum AccessMode {
    ReadOnly,
    WriteOnly,
}

impl AccessMode {
    const fn into_bits(self) -> u64 {
        self as _
    }
    const fn from_bits(value: u32) -> Self {
        match value {
            0 => Self::ReadOnly,
            1 => Self::WriteOnly,
            _ => unimplemented!(),
        }
    }
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[repr(transparent)]
    pub struct StatusFlags: u32 {
        const CREATE   = 1 << 0;
        const TRUNCATE = 1 << 1;
        const APPEND   = 1 << 2;
        const NONBLOCK = 1 << 3;
    }
}

#[repr(u64)]
enum StdFd {
    Stdin,
    Stderr,
    Stdout,
}

pub type Fd = u64;

pub struct File {
    pub inode: Arc<INode>,

    pub pos: usize,
    pub flags: OpenFlags,

    pub ops: Arc<dyn FileOps>,
}

impl File {
    pub fn status_flags(&self) -> StatusFlags {
        StatusFlags::from_bits_truncate(self.flags.status())
    }
    pub fn set_status(&mut self, status: StatusFlags) {
        self.flags.set_status(status.bits());
    }
}

impl Drop for File {
    fn drop(&mut self) {
        self.ops.close(self)
    }
}

#[bitfield(u64)]
pub struct OpenFlags {
    #[bits(32)]
    pub access_mode: AccessMode,
    #[bits(32)]
    pub status: u32,
}

#[derive(Debug)]
pub struct Stat {
    pub dev: u64,       /* ID of device containing file */
    pub ino: u64,       /* inode number */
    pub mode: NodeType, /* inode type */
    pub rdev: u64,      /* device ID (if special file) */
    pub nlink: usize,   /* always 1 on fat32 */
    pub size: usize,    /* total size, in bytes */
    pub blksize: u64,   /* blocksize for file system I/O */
    pub blocks: u64,    /* number of 512B blocks allocated */
    pub mtime: u64,     /* time of last modification */
}

impl Default for Stat {
    fn default() -> Self {
        Self {
            dev: 0,
            ino: 0,
            mode: NodeType::CharDevice,
            rdev: 0,
            nlink: 0,
            size: 0,
            blksize: 0,
            blocks: 0,
            mtime: 0,
        }
    }
}
pub fn full_path(dentry: Arc<RwLock<DEntry>>) -> String {
    let mut segments = vec![];
    let mut curr = dentry;
    loop {
        let (name, parent_weak) = {
            let guard = curr.read();
            (guard.name.clone(), guard.parent.clone())
        };

        segments.push(name);
        if let Some(parent) = parent_weak {
            if let Some(parent) = parent.upgrade() {
                curr = parent;
                continue;
            }
        }

        break;
    }
    segments.reverse();

    format!("/{}", &segments[1..].join("/"))
}

// NOTE: ensure sanitized to absolute paths before passed to VFS layer
fn find_mountpoint(path: &str) -> (&Mount, String) {
    let table = MOUNT_TABLE.try_get().unwrap();
    let mut curr_path = path;

    serial_println!("{:#?}", table.keys().collect::<Vec<&String>>());

    while !curr_path.is_empty() {
        let last_slash = curr_path.rfind('/').unwrap();
        let (parent, _) = curr_path.split_at(last_slash);
        if let Some(mp) = table.get(parent) {
            return (mp, parent.into());
        }
        curr_path = parent;
    }

    (table.get("/").unwrap(), "/".into())
}

pub fn get_root_dentry() -> Arc<RwLock<DEntry>> {
    MOUNT_TABLE.get().unwrap().get("/").unwrap().root.clone()
}

pub fn find_parent_dentry(path: &str) -> Option<Arc<RwLock<DEntry>>> {
    let (parent, _) = path.rsplit_once('/').unwrap();
    find_dentry(parent)
}

pub fn find_dentry(path: &str) -> Option<Arc<RwLock<DEntry>>> {
    // TODO: make sure path is abs or relative, if relative, combine with CWD
    let mut path = path.trim().to_string();

    if !path.starts_with("/") {
        // relative
        path = join_path_with_cwd(&path);
    }

    let (mount, mp_root_path) = find_mountpoint(&path);

    let mut current = mount.root.clone();

    let rel_path = if mp_root_path == "/" {
        &path
    } else {
        path.strip_prefix(&mp_root_path).unwrap_or(&path)
    };

    for segment in rel_path
        .trim_start_matches('/')
        .split('/')
        .filter(|p| !p.is_empty())
    {
        let child_cached = {
            let main_guard = current.read();
            let child_guard = main_guard.children.read();
            child_guard.get(segment).cloned()
        };
        if let Some(child) = child_cached {
            current = child;
        } else {
            // lookup
            let disk_lookup = {
                let main_guard = current.read();
                main_guard.inode.ops.lookup(&main_guard.inode, segment)
            };
            if let Some(node) = disk_lookup {
                let dentry = Arc::new(RwLock::new(DEntry {
                    name: segment.into(),
                    inode: node.into(),
                    parent: Some(Arc::downgrade(&current)),
                    children: RwLock::new(BTreeMap::new()),
                }));
                let dentry_clone = dentry.clone();
                // add it to parent cache
                current
                    .read()
                    .children
                    .write()
                    .insert(segment.into(), dentry);

                current = dentry_clone;
            } else {
                return None;
            }
        }
    }

    Some(current)
}

fn join_path_with_cwd(path: &str) -> String {
    let p = curr_proc();
    let p = p.lock();
    let cwd = full_path(p.cwd.clone());
    if cwd == "/" {
        format!("{cwd}{path}")
    } else {
        format!("{cwd}/{path}")
    }
}

pub fn mount_fat32(table: &mut MountTable, mount_path: &str) {
    let driver = AtaDriver::new(BusType::Primary, DriveType::Slave);
    let _ = driver.availability();
    const SECTOR: usize = 512;
    let mut bpb = vec![0u8; SECTOR];
    driver.read(0, 1, &mut bpb).unwrap();
    let bpb = unsafe { (*(bpb.as_ptr() as *const BPB)).clone() };
    let mut fs_info = vec![0u8; SECTOR];
    driver.read(bpb.fs_info as u64, 1, &mut fs_info).unwrap();
    let fs_info = unsafe { (*(fs_info.as_ptr() as *const FSInfo)).clone() };

    let fat_driver = Arc::new(Fat32::new(bpb, fs_info, driver));
    let fat_sb = Arc::new(SuperBlock {
        fs_type: FsType::Fat32,
    });
    let fat_root_inode: Arc<INode> = fat_driver.get_root_inode(fat_driver.clone(), fat_sb.clone());
    let fat_root_dent = Arc::new(RwLock::new(DEntry {
        name: "/".into(),
        inode: Arc::clone(&fat_root_inode),
        parent: None,
        children: RwLock::new(BTreeMap::new()),
    }));
    let fatfs = Mount {
        root: Arc::clone(&fat_root_dent),
        mountpoint: Arc::clone(&fat_root_dent),
        sb: fat_sb,
    };
    table.insert(mount_path.into(), fatfs);
}

pub fn mount_initramfs(table: &mut MountTable, mount_path: &str) {
    let ustar = USTAR::new(include_bytes!("../../../../target/ustarfs.tar"));
    let ustar_sb = Arc::new(SuperBlock {
        fs_type: FsType::Ustar,
    });
    let ustar_root_inode: Arc<INode> = ustar.get_root_inode();
    let ustar_root_dent = Arc::new(RwLock::new(DEntry {
        name: "/".into(),
        inode: Arc::clone(&ustar_root_inode),
        parent: None,
        children: RwLock::new(BTreeMap::new()),
    }));
    let ustarfs = Mount {
        root: Arc::clone(&ustar_root_dent),
        mountpoint: Arc::clone(&ustar_root_dent),
        sb: ustar_sb,
    };

    table.insert(mount_path.into(), ustarfs);
}

pub fn init_vfs() {
    let mut table: MountTable = MountTable::new();
    mount_fat32(&mut table, "/");
    // mount_initramfs(&mut table, "/init");

    table.insert("/dev".into(), mount_devfs());

    MOUNT_TABLE.init_once(|| table);
}
