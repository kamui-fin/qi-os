# Roadmap

- devfs
    - /dev/zero
    - /dev/null
    - /dev/urandom
    - /dev/mouse
    - /dev/keyboard
    - /dev/stdin, /dev/stdout, /dev/stderr
- unit + integration testing setup
    - Kernel debugger (quit & dump useful info on key press)
- TTY:
    - tty1: read-only log buffer
        - /dev/console = /dev/tty0
    - tty2: normal terminal
        - /dev/tty1
- Shell
    - env var

- buddy + slab allocator
- more concurrency:
    - SMP (for engine)
        - LAPIC/IOAPIC/ACPI
    - mutex / semaphore APIs
    - pthreads API + TLS

- Compositing:
    - Wallpaper
    - Floating windows
    - Borders

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
