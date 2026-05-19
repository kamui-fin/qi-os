# Roadmap

- Parallelism:
    - SMP (for engine)
        - LAPIC/IOAPIC/ACPI
    - mutex / semaphore APIs
    - pthreads API + TLS
- Complete shell functionality

--- 

- squash out all deadlocks & bottlenecks
- proper error handling
- wait queues
- buddy + slab allocator
- unit + integration testing setup
    - Kernel debugger (quit & dump useful info on key press)
- compositor:
    - wallpaper
    - bar
    - borders + shadows + window btns
    - cursor
    - floating windows

---

Wayyy into future:
- port doom
- UNIX System V IPC
- tmpfs + ext2
- filesystem buffering
- more advanced scheduler
- stack smash protector
- run llm on cpu
- multiboot support
- TCP/IP stack
