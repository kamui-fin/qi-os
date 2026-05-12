use core::ffi::CStr;

use alloc::sync::Arc;
use spin::Mutex;

use crate::{
    driver::cmos::get_unix_time,
    fs::vfs::{find_dentry, find_parent_dentry, Fd, File, OpenFlags, StatusFlags},
    serial_println,
    task::proc::curr_proc,
};

pub fn sys_write(fd: Fd, buf: &[u8]) -> usize {
    let file = {
        let p = curr_proc();
        let mut p = p.lock();
        p.fd.get_mut(fd as usize).unwrap().clone()
    };

    let mut file = file.lock();

    let bytes_written = file.ops.write(&file, buf);
    file.pos += bytes_written;

    let mut meta = file.inode.meta.lock();
    meta.mtime = get_unix_time() as u64;
    meta.dirty = true;
    bytes_written
}

pub fn sys_read(fd: Fd, buf: &mut [u8]) -> usize {
    let file = {
        let p = curr_proc();
        let p = p.lock();
        p.fd.get(fd as usize).unwrap().clone()
    };

    let mut file_guard = file.lock();
    let bytes_read = file_guard.ops.read(&file_guard, buf);
    serial_println!("[vfs] read {bytes_read} bytes");
    file_guard.pos += bytes_read;
    bytes_read
}

pub fn sys_open(path_addr: usize, flags: OpenFlags) -> Option<Fd> {
    let path = unsafe { CStr::from_ptr(path_addr as *const i8) };
    let path = path.to_str().unwrap();

    let dentry = find_dentry(path);

    if dentry.is_none()
        && StatusFlags::from_bits_truncate(flags.status()).contains(StatusFlags::CREATE)
    {
        let d_parent = find_parent_dentry(path).unwrap();
        let d_parent_guard = d_parent.read();
        let i_parent = d_parent_guard.inode.clone();
        i_parent
            .ops
            .create_file(&i_parent, path.rsplit_once("/").unwrap().1);
    }

    if let Some(inode) = dentry.map(|d| d.read().inode.clone()) {
        let ops = inode.ops.open(&inode, flags);
        let file = Arc::new(Mutex::new(File {
            inode,
            pos: 0,
            flags,
            ops,
        }));

        let fd = {
            let p = curr_proc();
            let mut p = p.lock();

            let entry = p.fd.vacant_key();
            p.fd.insert(file);
            entry as u64
        };

        return Some(fd);
    }

    None
}

pub fn sys_close(fd: Fd) {
    let p = curr_proc();
    let mut p = p.lock();
    p.fd.remove(fd as usize);
}
