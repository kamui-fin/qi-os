# Roadmap

- VFS
- Stack smash protector
- Unit + Integraiton testing setup
- TTY:
    - tty1: read-only log buffer
    - tty2: normal terminal
- More concurrency:
    - SMP (for engine)
        - LAPIC/IOAPIC/ACPI
    - mutex / semaphore APIs
    - pthreads API + TLS
- IPC
    - Shared memory 
    - Message passing, streams, or sockets
- Shell
- Compositing:
    - Wallpaper
    - Floating windows
    - Borders

Wayyy into future:
- Sound
- TCP/IP stack

Pending:
- Kernel debugger (quit & dump useful info on key press)
    -> see https://gitlab.com/bztsrc/minidbg
