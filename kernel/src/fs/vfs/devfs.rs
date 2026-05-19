use alloc::{collections::btree_map::BTreeMap, string::ToString, sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use spin::{Mutex, RwLock};

use crate::{
    console::TtyDeviceHandle,
    driver::serial::serial_write,
    fs::vfs::{
        find_dentry, DEntry, DEntryMinimal, File, FileOps, FsMetadata, FsType, INode, INodeData,
        INodeOps, Mount, MountTable, NodeType, SuperBlock,
    },
    random::{get_rand_range, mix_entropy, mix_entropy_with},
    task::mouse::MouseDeviceHandle,
    tty::TTY,
};

// major: device
pub static DEVICE_TABLE: OnceCell<Mutex<BTreeMap<u8, Device>>> = OnceCell::uninit();

pub struct Device {
    ops: Arc<dyn FileOps>,
}

impl INodeOps for Device {
    fn open(&self, _: &INode, _: super::OpenFlags) -> Arc<dyn FileOps> {
        self.ops.clone()
    }
}

struct Zero;
impl FileOps for Zero {
    fn read(&self, _: &File, buffer: &mut [u8]) -> usize {
        buffer.fill(0);
        buffer.len()
    }
    fn write(&self, _: &File, buffer: &[u8]) -> usize {
        buffer.len()
    }
}

struct Null;
impl FileOps for Null {
    fn read(&self, _: &File, buffer: &mut [u8]) -> usize {
        0
    }
    fn write(&self, _: &File, buffer: &[u8]) -> usize {
        buffer.len()
    }
}

struct URandom;
impl FileOps for URandom {
    fn read(&self, _: &File, buffer: &mut [u8]) -> usize {
        for i in 0..buffer.len() {
            buffer[i] = get_rand_range(0, 255) as u8;
        }
        buffer.len()
    }
    fn write(&self, _: &File, buffer: &[u8]) -> usize {
        // add data into entropy
        let mut result: Vec<u64> = buffer
            .chunks_exact(8)
            .map(|c| u64::from_be_bytes(c.try_into().unwrap()))
            .collect();
        let remainder = buffer.chunks_exact(8).remainder();
        if !remainder.is_empty() {
            let mut padded = [0; 8];
            padded[..remainder.len()].copy_from_slice(remainder);
            result.push(u64::from_be_bytes(padded));
        }

        for entropy in result {
            mix_entropy_with(entropy);
        }

        buffer.len()
    }
}
struct SerialDeviceHandle {}

impl FileOps for SerialDeviceHandle {
    fn read(&self, _: &File, _buffer: &mut [u8]) -> usize {
        0
    }

    fn write(&self, _: &File, buffer: &[u8]) -> usize {
        serial_write(buffer);
        buffer.len()
    }
}

struct DevFsDir;

impl INodeOps for DevFsDir {
    fn open(&self, inode: &INode, flags: super::OpenFlags) -> Arc<dyn FileOps> {
        // TODO: For now we have no need for minor
        let INodeData::Device { major, minor } = &inode.data else {
            panic!();
        };
        DEVICE_TABLE.get().unwrap().lock()[major].ops.clone()
    }

    fn lookup(&self, dir: &INode, name: &str) -> Option<Arc<INode>> {
        let INodeData::DevFs(devnode_map) = &dir.data else {
            panic!();
        };

        devnode_map.lock().get(name).map(|i| i.clone())
    }

    fn readdir(&self, dir: &INode) -> Vec<super::DEntryMinimal> {
        let INodeData::DevFs(devnode_map) = &dir.data else {
            panic!();
        };

        devnode_map
            .lock()
            .iter()
            .map(|e| {
                let mut name_buf = [0u8; 64];
                let e_name = e.0.clone();
                let name_bytes = e_name.as_bytes();
                let copy_len = name_bytes.len().min(64);
                name_buf[..copy_len].copy_from_slice(&name_bytes[..copy_len]);

                DEntryMinimal {
                    name: name_buf,
                    inum: e.1.inum,
                    size: e.1.meta.lock().size,
                    filetype: NodeType::CharDevice,
                }
            })
            .collect()
    }
}

pub fn mount_devfs() -> Mount {
    let sb = Arc::new(SuperBlock {
        fs_type: FsType::DevFs,
    });

    let zero_inode = Arc::new(INode {
        inum: 1,
        fs: sb.clone(),
        mode: NodeType::CharDevice,
        data: INodeData::Device { major: 1, minor: 0 },
        meta: Mutex::new(FsMetadata {
            size: 0,
            mtime: 0,
            dirty: false,
        }),
        ops: Arc::new(Device {
            ops: Arc::new(Zero),
        }),
    });
    let null_inode = Arc::new(INode {
        inum: 2,
        fs: sb.clone(),
        mode: NodeType::CharDevice,
        data: INodeData::Device { major: 2, minor: 0 },
        meta: Mutex::new(FsMetadata {
            size: 0,
            mtime: 0,
            dirty: false,
        }),
        ops: Arc::new(Device {
            ops: Arc::new(Null),
        }),
    });
    let rand_inode = Arc::new(INode {
        inum: 3,
        fs: sb.clone(),
        mode: NodeType::CharDevice,
        data: INodeData::Device { major: 3, minor: 0 },
        meta: Mutex::new(FsMetadata {
            size: 0,
            mtime: 0,
            dirty: false,
        }),
        ops: Arc::new(Device {
            ops: Arc::new(URandom),
        }),
    });

    let tty_one_inode = Arc::new(INode {
        inum: 4,
        fs: sb.clone(),
        mode: NodeType::CharDevice,
        data: INodeData::Device { major: 4, minor: 0 },
        meta: Mutex::new(FsMetadata {
            size: 0,
            mtime: 0,
            dirty: false,
        }),
        ops: Arc::new(Device {
            ops: Arc::new(TtyDeviceHandle { tty_id: 1 }),
        }),
    });

    let mouse_inode = Arc::new(INode {
        inum: 5,
        fs: sb.clone(),
        mode: NodeType::CharDevice,
        data: INodeData::Device { major: 5, minor: 0 },
        meta: Mutex::new(FsMetadata {
            size: 0,
            mtime: 0,
            dirty: false,
        }),
        ops: Arc::new(Device {
            ops: Arc::new(MouseDeviceHandle {}),
        }),
    });

    let serial_inode = Arc::new(INode {
        inum: 6,
        fs: sb.clone(),
        mode: NodeType::CharDevice,
        data: INodeData::Device { major: 6, minor: 0 },
        meta: Mutex::new(FsMetadata {
            size: 0,
            mtime: 0,
            dirty: false,
        }),
        ops: Arc::new(Device {
            ops: Arc::new(SerialDeviceHandle {}),
        }),
    });

    let mut devnode_map = BTreeMap::new();
    devnode_map.insert("zero".to_string(), zero_inode);
    devnode_map.insert("null".to_string(), null_inode);
    devnode_map.insert("urandom".to_string(), rand_inode);

    devnode_map.insert("tty1".to_string(), tty_one_inode);

    devnode_map.insert("mouse".to_string(), mouse_inode);

    devnode_map.insert("serial".to_string(), serial_inode);

    let root_inode = Arc::new(INode {
        inum: 0,
        fs: sb.clone(),
        mode: NodeType::Directory,
        data: INodeData::DevFs(Mutex::new(devnode_map)),
        meta: Mutex::new(FsMetadata {
            size: 0,
            mtime: 0,
            dirty: false,
        }),
        ops: Arc::new(DevFsDir),
    });
    let root_dent = Arc::new(RwLock::new(DEntry {
        name: "/".into(),
        parent: None,
        children: RwLock::new(BTreeMap::new()),
        inode: root_inode,
    }));

    let devfs = Mount {
        root: Arc::clone(&root_dent),
        mountpoint: Arc::clone(&root_dent),
        sb: sb.clone(),
    };

    devfs
    // TODO:
    //     - /dev/keyboard
}
