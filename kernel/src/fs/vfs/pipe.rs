use core::sync::atomic::AtomicU64;

use crate::{
    spinlock::Spinlock,
    task::{
        scheduler::{block_task_with_lock, SCHEDULER},
        thread::ThreadControlBlock,
    },
};
use alloc::{sync::Arc, vec::Vec};
use crossbeam_queue::ArrayQueue;

use crate::{
    fs::vfs::{File, FileOps, FsType, INode, INodeData, INodeOps, OpenFlags, Stat, SuperBlock},
    task::thread::{BlockReason, ThreadState},
    PROC,
};

const PIPE_BUF: usize = 1024;

lazy_static::lazy_static! {
    pub static ref PIPE_FS: Arc<SuperBlock> = Arc::new(SuperBlock {
        fs_type: FsType::PipeFs,
    });
}

pub static PIPE_ID_COUNT: AtomicU64 = AtomicU64::new(0);

pub struct Pipe {
    pub buffer: Spinlock<ArrayQueue<u8>>,
    pub readers: usize,
    pub writers: usize,
}

pub struct PipeInodeOps;
pub struct PipeOps;

impl INodeOps for PipeInodeOps {
    fn open(&self, inode: &INode, flags: OpenFlags) -> Arc<dyn FileOps> {
        Arc::new(PipeOps)
    }

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
            size: pipe.buffer.lock().len(),
            blksize: 4096,
            blocks: 0,
            mtime: meta.mtime,
        }
    }
}

impl FileOps for PipeOps {
    fn read(&self, file: &File, buffer: &mut [u8]) -> usize {
        let node = &file.inode;
        let pipe_id = node.inum;
        let INodeData::Pipe(pipe) = &node.data else {
            panic!("expected pipe data");
        };

        let mut bytes_read = 0;
        loop {
            let guard = pipe.buffer.lock();

            if !guard.is_empty() {
                while let Some(byte) = guard.pop() {
                    if bytes_read >= buffer.len() {
                        break;
                    }
                    buffer[bytes_read] = byte;
                    bytes_read += 1;
                }

                break;
            }

            if pipe.writers == 0 {
                return 0; // eof if all write ends closed
            }

            // sleep throw into wait queue
            block_task_with_lock(BlockReason::WaitPipeRead(pipe_id), guard, &pipe.buffer);
        }

        if bytes_read > 0 {
            wake_pipe_sleepers(pipe_id, Direction::Write);
        }

        bytes_read
    }

    fn write(&self, file: &File, buffer: &[u8]) -> usize {
        let node = &file.inode;
        let pipe_id = node.inum;
        let INodeData::Pipe(pipe) = &node.data else {
            panic!("expected pipe data");
        };

        let mut bytes_write = 0;

        for chunk in buffer.chunks(PIPE_BUF) {
            loop {
                let guard = pipe.buffer.lock();
                if chunk.len() + guard.len() > guard.capacity() {
                    if pipe.readers == 0 {
                        return bytes_write;
                    }
                    block_task_with_lock(BlockReason::WaitPipeWrite(pipe_id), guard, &pipe.buffer);
                } else {
                    for byte in chunk {
                        guard.push(*byte);
                        bytes_write += 1;
                    }

                    drop(guard);

                    wake_pipe_sleepers(pipe_id, Direction::Read);

                    break;
                }
            }
        }

        bytes_write
    }
}

enum Direction {
    Read,
    Write,
}

// TODO: obviously this is O(n) but its a quick n dirty
fn wake_pipe_sleepers(pipe_id: u64, dir: Direction) {
    let procs = PROC.get().unwrap().lock();
    let mut to_wake = Vec::new();
    let mut sched = SCHEDULER.lock();

    for proc in procs.iter() {
        let pid = proc.lock().pid;
        let thread = sched.threads.iter().find(|t| t.id == pid).unwrap();

        match (&dir, &thread.state) {
            (Direction::Read, ThreadState::Blocked(BlockReason::WaitPipeRead(waiting_on)))
            | (Direction::Write, ThreadState::Blocked(BlockReason::WaitPipeWrite(waiting_on))) => {
                if *waiting_on == pipe_id {
                    to_wake.push(thread.id);
                }
            }
            _ => {}
        };
    }
    for thread_id in to_wake {
        sched.unblock_task(thread_id);
    }
}
