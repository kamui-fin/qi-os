// **TTY Driver**
// - Input handling (keyboard)
// - Output side
// - Line discipline

use core::sync::atomic::AtomicBool;

use crate::spinlock::Spinlock;
use crate::task::scheduler::{SCHEDULER, block_task};
use crate::{
    console::{wake_tty_renderer, Color, ColorCode, ScreenChar, VirtualTerminal},
    fs::vfs::FileOps,
    print, serial_print, serial_println,
    task::{
        keyboard::ScancodeStream,
        thread::{BlockReason, ThreadState},
        tty::{self},
    },
    PROC,
};
use alloc::{sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use futures_util::stream::StreamExt;
use pc_keyboard::{layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1};

// TODO:
// - Backspace

// For now, we'll only support canonical mode
// - buffered until enter
// - line editing
// - echoing
pub struct TTY {
    id: u8,
    // TODO: this will be needed when PTYs are introduced for gui term
    // for now, we'll be coupling VirtualTerminal
    // output_char_queue: ArrayQueue<ScreenChar>,
    completed_input_queue: ArrayQueue<u8>,
    line_buf: Spinlock<Vec<u8>>,
    is_eof: AtomicBool,

    pub terminal: Spinlock<VirtualTerminal>,
}

impl TTY {
    pub fn new(id: u8) -> Self {
        Self {
            id,
            completed_input_queue: ArrayQueue::new(80 * 40),
            line_buf: Spinlock::new(Vec::with_capacity(80)),
            is_eof: AtomicBool::new(false),

            terminal: Spinlock::new(VirtualTerminal::new()),
        }
    }

    fn input_byte(&self, byte: u8) {
        self.line_buf.lock().push(byte);
    }

    pub fn output_byte(&self, byte: u8) {
        self.terminal.lock().write_byte(byte);
    }

    // returns how many bytes were written to output
    pub fn handle_key(&self, key: DecodedKey) -> bool {
        // echoing input needs to update tty display
        let mut wrote_bytes = false;

        match key {
            DecodedKey::Unicode(character) => {
                match character {
                    '\u{8}' => {
                        // delete last char from line buffer
                        if let Some(_) = self.line_buf.lock().pop() {
                            // append \b \b to output (visual deletion)
                            // \x08 is moving cursor one back
                            // " " overwrites with space
                            self.write("\x08 \x08");
                            wrote_bytes = true;
                        }
                    }
                    '\n' => {
                        // \n - done with line
                        serial_println!("Done with line!");
                        self.input_byte(b'\n');
                        self.output_byte(b'\n');

                        self.commit_line();
                        wrote_bytes = true;
                    }
                    // ctrl+c
                    '\u{3}' => {
                        self.write("^C");
                        wrote_bytes = true;
                    }
                    // ctrl+d
                    '\u{4}' => {
                        // eof
                        if self.line_buf.lock().is_empty() {
                            self.is_eof
                                .store(true, core::sync::atomic::Ordering::SeqCst);
                        } else {
                            self.commit_line();
                        }
                    }
                    _ => {
                        self.input_byte(character as u8);
                        self.output_byte(character as u8);
                        wrote_bytes = true;
                    }
                }
            }
            DecodedKey::RawKey(key) => match key {
                _ => {}
            },
        }

        wrote_bytes
    }

    // for stdout and stderr
    pub fn write(&self, s: &str) -> usize {
        for byte in s.bytes() {
            self.output_byte(byte);
        }

        s.len()
    }

    // for stdin
    pub fn read(&self, buffer: &mut [u8]) -> usize {
        loop {
            // TODO: atomic sleep bug. Move to wait queues ASAP
            if !self.completed_input_queue.is_empty() {
                break;
            }
            if self.is_eof.load(core::sync::atomic::Ordering::SeqCst) {
                self.is_eof
                    .store(false, core::sync::atomic::Ordering::SeqCst);
                return 0;
            }
            // wake me up when you commit line
            block_task(BlockReason::WaitStdin(self.id));
        }

        // FIXME: we never get here?
        serial_println!("Actually starting to read");

        let mut i = 0;
        while let Some(c) = self.completed_input_queue.pop() {
            if i >= buffer.len() {
                break;
            }
            buffer[i] = c;
            i += 1
        }

        return i;
    }

    fn commit_line(&self) {
        for char in self.line_buf.lock().iter() {
            self.completed_input_queue.force_push(*char);
        }
        self.line_buf.lock().clear();
        self.wakeup_readers();
    }

    // TODO: very primitive, switch to wait queue ASAP
    fn wakeup_readers(&self) {
        let mut sched = SCHEDULER.lock();
        let readers_to_wake = {
            let mut threads = Vec::new();
            let procs = PROC.get().unwrap().lock();
            for proc in procs.iter() {
                let pid = proc.lock().pid;
                let thread = sched.threads.iter().find(|t| t.id == pid).unwrap();
                if let ThreadState::Blocked(BlockReason::WaitStdin(tty_num)) = thread.state {
                    if self.id == tty_num {
                        threads.push(thread.id);
                    }
                }
            }

            threads
        };
        for id in readers_to_wake {
            sched.unblock_task(id);
        }
    }
}
