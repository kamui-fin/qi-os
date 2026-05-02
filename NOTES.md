# Roadmap

- env var
- devfs

- unit + integration testing setup
- more concurrency:
    - SMP (for engine)
        - LAPIC/IOAPIC/ACPI
    - mutex / semaphore APIs
    - pthreads API + TLS
- buddy + slab allocator
- tmpfs

- TTY:
    - tty1: read-only log buffer
    - tty2: normal terminal
- Shell
- Compositing:
    - Wallpaper
    - Floating windows
    - Borders

Wayyy into future:
- stack smash protector
- Sound
- TCP/IP stack

Pending:
- Kernel debugger (quit & dump useful info on key press)
    -> see https://gitlab.com/bztsrc/minidbg
