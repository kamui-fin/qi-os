// Rough outline:
// 1. detect cpus
// 2. start APs
// 3. give each cpu its own scheduler state / kernel stack
// 4. add locking
// 5. enable timer interrupts on each CPU
//
//

/* ACPI MADT tells you:
how many CPUs exist
LAPIC IDs
IOAPIC location */

// each cpu needs:
// current thread
/* kernel stack
TSS
scheduler local state
run queue (optional initially) */
