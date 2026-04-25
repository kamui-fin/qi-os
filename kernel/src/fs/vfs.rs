use alloc::boxed::Box;
use alloc::string::String;
use alloc::{rc::Weak, sync::Arc};
use bitflags::bitflags;
use spin::RwLock;

enum FsType {
    Fat32,
    Ustar,
}

enum MountSource {
    Device(String),
    NoDev(String),
}

// Metadata about a specific mounted file system (total size, block size, etc.).
struct SuperBlock {
    fs_type: FsType,
    magic: u64,

    num_blocks: usize,
    num_inodes: usize,

    root_inode: Arc<INode>,

    device: MountSource,
    mountpoint: Arc<DEntry>,
}

pub trait SuperblockOperations {
    fn alloc_inode();
    fn destroy_inode();
    fn write_inode();
    fn sync_fs();
    fn statfs();
}

enum Mode {
    File,
    Directory,
}

// Represents a physical file on the disk. It holds metadata (permissions, owner) but not the filename.
struct INode {
    inum: u64,
    fs: Arc<SuperBlock>,

    size: usize,
    mode: Mode, // file, dir, etc.
    nlink: u64,

    atime: u64, // access time
    mtime: u64, // content OR metadata changes

                /* i_op: Box<dyn INodeOperations>,
                f_op: Box<dyn FileOperations>, */
}

pub trait INodeOperations {
    fn create();
    fn lookup();
    fn rename();
    fn mkdir();
    fn rmdir();
    fn link();
    fn unlink();

    /* fn mknod();
    fn getattr();
    fn setattr(); */
}

// (Directory Entry). Links an Inode to a name. This is how the system resolves paths like /home/user.
struct DEntry {
    name: String,
    inode: Arc<INode>,
    parent: Weak<RwLock<DEntry>>,
    mount_structure: Option<Arc<MountPoint>>,
    flags: u8,
}

struct MountPoint {
    source: Arc<DEntry>,
    root: Arc<DEntry>,
    sb: SuperBlock,
}

type MountTable = alloc::collections::BTreeMap<String, MountPoint>;

enum OpenFlag {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

/*
* File operations:
* - llseek,
* - read / write,
* - iterate,
* - poll,
* - unlocked_ioctl,
* - mmap,
* - open,
* - release,
* - fsync
*/

// Represents a file opened by a process. It tracks the current offset (where the cursor is) and the mode (read/write).
struct File {
    inode: Arc<INode>,
    position: usize,
    flag: OpenFlag,
}

pub trait FileOperations {
    /* fn llseek();
    fn iterate();
    fn poll();
    fn unlocked_ioctl();
    fn mmap();
    fn fsync(); */

    fn read();
    fn write();
    fn open();
    fn release();
}
