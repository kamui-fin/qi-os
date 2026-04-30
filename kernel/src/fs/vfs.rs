use core::any::Any;
use core::ffi::CStr;
use core::sync::atomic::AtomicU64;

use alloc::boxed::Box;
use alloc::collections::btree_map::BTreeMap;
use alloc::ffi::c_str;
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use bitfield_struct::bitfield;
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use lazy_static;
use spin::{Mutex, RwLock};

use crate::driver::ata::{AtaDriver, BusType, DriveType};
use crate::driver::cmos::get_unix_time;
use crate::fs::fat::{BlockDevice, DirEntryWithLoc, FSInfo, Fat32, BPB};
use crate::fs::ustar::USTAR;
use crate::task::proc::{with_curr_proc, with_curr_proc_mut};

lazy_static::lazy_static! {
    pub static ref PIPE_FS: Arc<SuperBlock> = Arc::new(SuperBlock {
        fs_type: FsType::PipeFs,
    });
}

pub enum FsType {
    Fat32,
    Ustar,
    PipeFs,
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
    FatFs(Mutex<DirEntryWithLoc>),
}

pub struct INode {
    pub inum: u64,
    pub fs: Arc<SuperBlock>,

    // these two MUST stay in sync
    pub mode: NodeType,
    pub data: INodeData,

    pub meta: Mutex<FsMetadata>,

    pub ops: Arc<dyn INodeOps>,
}

pub struct FsMetadata {
    pub size: usize,
    pub mtime: u64, // content OR metadata changes
}

pub trait FileOps: Send + Sync {
    fn read(&self, file: &File, buffer: &mut [u8]) -> usize;
    fn write(&self, file: &File, buffer: &[u8]) -> usize;
}

pub trait INodeOps: Send + Sync {
    fn read(&self, inode: &INode, offset: usize, buf: &mut [u8]) -> usize;
    fn write(&self, inode: &INode, offset: usize, buf: &[u8]) -> usize;
    fn lookup(&self, parent: &INode, name: &str) -> Option<INode>;
    fn readdir(&self, dir: &INode) -> Vec<DEntryMinimal>;
    fn create_file(&self, dir: &INode, name: &str);
    fn rename(&self, node: &INode, to: &str);
    fn mkdir(&self, dir: &INode, name: &str);
    fn delete_file(&self, file: &INode);
    fn rmdir(&self, dir: &INode);
    fn stat(&self, node: &INode) -> Stat;
}

pub struct DEntry {
    pub name: String,
    pub inode: Arc<INode>,
    pub parent: Option<Weak<RwLock<DEntry>>>, // None if root
    // cache of children we alr looked up
    pub children: RwLock<BTreeMap<String, Arc<RwLock<DEntry>>>>,
}

pub struct DEntryMinimal {
    pub name: String,
    pub inum: u32,
    pub filetype: NodeType,
    pub size: usize,
}

pub static MOUNT_TABLE: OnceCell<MountTable> = OnceCell::uninit();

pub struct Mount {
    pub root: Arc<RwLock<DEntry>>,
    pub mountpoint: Arc<RwLock<DEntry>>,
    pub sb: Arc<SuperBlock>,
}

pub type MountTable = alloc::collections::BTreeMap<String, Mount>;

#[derive(Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum AccessMode {
    ReadOnly,
    WriteOnly,
    AppendOnly,
}

impl AccessMode {
    const fn into_bits(self) -> u64 {
        self as _
    }
    const fn from_bits(value: u64) -> Self {
        match value {
            0 => Self::ReadOnly,
            1 => Self::WriteOnly,
            _ => Self::AppendOnly,
        }
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

    pub pos: Mutex<usize>,
    pub flag: AccessMode,

    pub ops: Arc<dyn FileOps>,
}

pub static PIPE_ID_COUNT: AtomicU64 = AtomicU64::new(0);

pub struct Pipe {
    pub buffer: ArrayQueue<u8>,
    pub readers: usize,
    pub writers: usize,
    /* rd_queue: WaitQueue,
    wr_queue: WaitQueue, */
}

pub struct Device {
    ops: Arc<dyn FileOps>,
}

pub static DEVICE_TABLE: OnceCell<Vec<Device>> = OnceCell::uninit();

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

pub struct PipeInodeOps;
pub struct PipeOps;

impl INodeOps for PipeInodeOps {
    fn read(&self, inode: &INode, offset: usize, buf: &mut [u8]) -> usize {
        0
    }
    fn write(&self, inode: &INode, offset: usize, buf: &[u8]) -> usize {
        0
    }
    fn lookup(&self, parent: &INode, name: &str) -> Option<INode> {
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
        let INodeData::Pipe(pipe) = &node.data else {
            panic!("expected pipe data");
        };
        let meta = node.meta.lock();
        Stat {
            dev: 0,
            ino: node.inum,
            mode: node.mode,
            rdev: 0,
            nlink: 1,
            size: pipe.buffer.len(),
            blksize: 4096,
            blocks: 0,
            mtime: meta.mtime,
        }
    }
}

// NOTE: ensure sanitized to absolute paths before passed to VFS layer
fn find_mountpoint(path: &str) -> &Mount {
    let table = MOUNT_TABLE.try_get().unwrap();
    let mut curr_path = path;

    while !curr_path.is_empty() {
        let last_slash = curr_path.rfind('/').unwrap();
        let (parent, _) = curr_path.split_at(last_slash);
        if let Some(mp) = table.get(parent) {
            return mp;
        }
        curr_path = parent;
    }

    table.get("/").unwrap()
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
    let mount = find_mountpoint(path);
    let mut current = mount.root.clone();
    for segment in path[1..].split('/').filter(|p| !p.is_empty()) {
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

pub fn sys_write(fd: Fd, buf: &[u8]) -> usize {
    with_curr_proc_mut(|p| {
        let file = p.fd.get_mut(fd as usize).unwrap().clone();
        let mut pos = file.pos.lock();

        let size = file.inode.meta.lock().size;
        let val_to_add;
        if let AccessMode::AppendOnly = file.flag {
            val_to_add = size + buf.len();
            file.inode
                .ops
                .write(&file.inode, file.inode.meta.lock().size, buf);

            file.inode.meta.lock().size += buf.len();

            *pos = val_to_add;
        } else {
            // TODO: allow write to exceed file size
            val_to_add = if *pos + buf.len() >= size {
                size.saturating_sub(*pos)
            } else {
                buf.len()
            };
            file.inode.ops.write(&file.inode, *pos, buf);
            *pos += val_to_add;
        }

        file.inode.meta.lock().mtime = get_unix_time() as u64;
        val_to_add
    })
}

pub fn sys_read(fd: Fd, buf: &mut [u8]) -> usize {
    with_curr_proc(|p| {
        let file = p.fd.get(fd as usize).unwrap().clone();
        file.inode.ops.read(&file.inode, *file.pos.lock(), buf);

        let mut pos = file.pos.lock();
        let size = file.inode.meta.lock().size;
        let val_to_add = if *pos + buf.len() >= size {
            size - *pos
        } else {
            buf.len()
        };
        *pos += val_to_add;
        val_to_add
    })
}

pub struct GenericFileOps;

impl FileOps for GenericFileOps {
    fn read(&self, file: &File, buffer: &mut [u8]) -> usize {
        file.inode.ops.read(&file.inode, *file.pos.lock(), buffer)
    }

    fn write(&self, file: &File, buffer: &[u8]) -> usize {
        file.inode.ops.write(&file.inode, *file.pos.lock(), buffer)
    }
}

impl FileOps for PipeOps {
    fn read(&self, file: &File, buffer: &mut [u8]) -> usize {
        let node = &file.inode;
        let INodeData::Pipe(pipe) = &node.data else {
            panic!("expected pipe data");
        };

        if pipe.buffer.is_empty() {
            if pipe.writers == 0 {
                return 0; // eof if all write ends closed
            }
            // sleep throw into wait queue
        }
    }

    fn write(&self, file: &File, buffer: &[u8]) -> usize {}
}

#[bitfield(u64)]
pub struct OpenFlags {
    #[bits(64)]
    pub access_mode: AccessMode,
}

pub fn sys_open(path_addr: usize, flag: OpenFlags) -> Option<Fd> {
    let path = unsafe { CStr::from_ptr(path_addr as *const i8) };
    let path = path.to_str().unwrap();

    let mp = find_mountpoint(path);
    // "/mnt/dir/file" -> "/dir/file"
    let mp_root_path = &mp.mountpoint.read().name;
    let rel_path = if mp_root_path == "/" {
        path
    } else {
        path.strip_prefix(mp_root_path).unwrap()
    };
    let mp_root = mp.root.read();
    let inode = mp_root.inode.ops.lookup(&mp_root.inode, rel_path);
    if let Some(inode) = inode {
        let file = Arc::new(File {
            inode: Arc::new(inode),
            pos: Mutex::new(0),
            flag: flag.access_mode(),
            ops: Arc::new(GenericFileOps),
        });

        let fd = with_curr_proc_mut(|p| {
            let entry = p.fd.vacant_key();
            p.fd.insert(file);
            entry as u64
        });

        return Some(fd);
    }

    None
}

// for now, all close does is delete File from PCB
pub fn sys_close(fd: Fd) {
    with_curr_proc_mut(|p| {
        p.fd.remove(fd as usize);
    });
}

// TODO:
//  - getdents
//  - lseek(fd, n)
//  - mount(fs, path)
//  - umount(path)
//  - stat(path)
//  - ...

pub fn mount_fat32(table: &mut MountTable, mount_path: &str) {
    let driver = AtaDriver::new(BusType::Primary, DriveType::Slave);
    let _ = driver.availability();
    const SECTOR: usize = 512;
    let mut bpb = vec![0u8; SECTOR];
    driver.read(0, 1, &mut bpb).unwrap();
    let bpb = unsafe { &*(bpb.as_ptr() as *const BPB) };
    let mut fs_info = vec![0u8; SECTOR];
    driver.read(bpb.fs_info as u64, 1, &mut fs_info).unwrap();
    let fs_info = unsafe { &*(fs_info.as_ptr() as *const FSInfo) };

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
    let ustar = USTAR::new(include_bytes!("../../../target/ustarfs.tar"));
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
    mount_initramfs(&mut table, "/init");

    MOUNT_TABLE.init_once(|| table);
}
