# Roadmap

- TTY:
    - tty1: read-only log buffer
        - /dev/console = /dev/tty0
    - tty2: normal terminal
        - /dev/tty1
- Shell
    - env var
- devfs
    - /dev/mouse
    - /dev/keyboard
- unit + integration testing setup
    - Kernel debugger (quit & dump useful info on key press)
- Compositing:
    - Wallpaper
    - Floating windows
    - Borders

- buddy + slab allocator
- wait queues
- more concurrency:
    - SMP (for engine)
        - LAPIC/IOAPIC/ACPI
    - mutex / semaphore APIs
    - pthreads API + TLS


Wayyy into future:
- multiboot support
- port doom
- UNIX System V IPC
- tmpfs + ext2
- filesystem buffering
- more advanced scheduler
- stack smash protector
- Sound
- TCP/IP stack
