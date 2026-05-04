// **TTY Driver**
// - Input handling (keyboard)
// - Output side
// - Line discipline

use core::sync::atomic::AtomicBool;

use crate::{
    console::{Color, ColorCode, ScreenChar, VirtualTerminal},
    fs::vfs::FileOps,
    print, serial_print,
    task::{
        keyboard::ScancodeStream,
        thread::{block_task, BlockReason, ThreadState, SCHEDULER},
        tty::{self},
    },
    PROC,
};
use alloc::{sync::Arc, vec::Vec};
use conquer_once::spin::OnceCell;
use crossbeam_queue::ArrayQueue;
use futures_util::stream::StreamExt;
use pc_keyboard::{layouts, DecodedKey, HandleControl, KeyCode, Keyboard, ScancodeSet1};
use spin::Mutex;

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
    line_buf: Mutex<Vec<u8>>,
    is_eof: AtomicBool,

    pub terminal: Mutex<VirtualTerminal>,
}

impl TTY {
    pub fn new(id: u8) -> Self {
        Self {
            id,
            completed_input_queue: ArrayQueue::new(80 * 40),
            line_buf: Mutex::new(Vec::with_capacity(80)),
            is_eof: AtomicBool::new(false),

            terminal: Mutex::new(VirtualTerminal::new()),
        }
    }

    fn input_byte(&self, byte: u8) {
        self.line_buf.lock().push(byte);
    }

    pub fn output_byte(&self, byte: u8) {
        self.terminal.lock().write_byte(byte);
    }

    pub fn handle_key(&self, key: DecodedKey) {
        match key {
            DecodedKey::Unicode(character) => {
                match character {
                    // ctrl+c
                    '\u{3}' => {}
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
                        self.output_byte(character as u8)
                    }
                }
            }
            DecodedKey::RawKey(key) => {
                if let pc_keyboard::KeyCode::LControl | pc_keyboard::KeyCode::RControl = key {
                    return;
                }
                match key {
                    pc_keyboard::KeyCode::Backspace => {
                        // delete last char from line buffer
                        if let Some(_) = self.line_buf.lock().pop() {
                            // append \b \b to output (visual deletion)
                            // \x08 is moving cursor one back
                            // " " overwrites with space
                            self.write("\x08 \x08");
                        }
                    }
                    pc_keyboard::KeyCode::Return => {
                        // \n - done with line
                        self.input_byte(b'\n');
                        self.output_byte(b'\n');

                        self.commit_line();
                    }
                    _ => {}
                }
            }
        }
    }

    // for stdout and stderr
    pub fn write(&self, s: &str) -> usize {
        for byte in s.bytes() {
            match byte {
                // printable ASCII byte or newline
                0x20..=0x7e | b'\n' => self.output_byte(byte),
                // not part of printable ASCII range
                _ => self.output_byte(0xfe),
            }
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
        let procs = PROC.get().unwrap().lock();
        for proc in procs.iter() {
            let thread = proc.tcb.lock();
            if let ThreadState::Blocked(BlockReason::WaitStdin(tty_num)) = thread.state {
                if self.id == tty_num {
                    sched.unblock_task(thread.id);
                }
            }
        }
    }
}
