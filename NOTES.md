# Roadmap

- SMP (for engine)
    - ~~LAPIC/IOAPIC/ACPI~~ 
    - ~~multi-core scheduler~~ (Rough draft is there but still debugging to do, will likely be rewritten)
    - More efficient scheduler: (work stealing, load balacing ect)
    - multi-core async executor?
- pci enumeration
- sound
- Complete shell functionality
- proper error handling

--- 

- unit + integration testing setup
    - Kernel debugger (quit & dump useful info on key press)
- stack smash protector
- wait queues
- buddy + slab allocator

---

Wayyy into future:
- advanced compositor:
    - wallpaper
    - floating windows
    - borders
    - workspaces
- user-space parallelism
    - pthreads + tls / sleeplock / mutex / semaphore APIs
- port doom
- UNIX System V IPC
- tmpfs + ext2
- filesystem buffering
- more advanced scheduler
- TCP/IP stack
- run llm on cpu
- multiboot support
- finish my nes emulator and port it
