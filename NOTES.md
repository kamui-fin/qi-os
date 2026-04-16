# Roadmap

- Kernel debugger (quit & dump useful info on key press)
    -> see https://gitlab.com/bztsrc/minidbg
- IPC
    - Shared memory 
    - Video memory access (compositor -> vram directly)
    - Message passing, streams, or sockets
- Compositing
- More concurrency:
    - pthreads API + TLS
    - mutex / semaphore APIs
    - SMP (for engine)
- Filesystem
    - USTAR on initrd into RAMFS. Test out general VFS API without relying on underlying filesystem
    - FAT
- Shell
- Sound
