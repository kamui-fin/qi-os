use core::sync::atomic::AtomicUsize;

use lazy_static;

use crate::driver::sound::hda::{PcmSample, PCA_BUFFER_VIRT_START};

#[derive(Debug, Clone)]
#[repr(C, packed)]
struct WavFormatHeader {
    riff_magic: [u8; 4],
    file_size: u32, // total file size - 8 bytes
    wave_magic: [u8; 4],

    // format chunk
    fmt_magic: [u8; 4],
    chunk_size: u32, // usually 16

    audio_format: u16,
    num_channels: u16,
    sample_rate: u32,
    byte_rate: u32,
    block_align: u16,
    bits_per_sample: u16,

    data_magic: [u8; 4], // data chunk header
    data_size: u32,
}

#[repr(align(2))]
struct AlignedWavData {
    data: [u8; include_bytes!("./sample.wav").len()],
}

static RAW_WAV: AlignedWavData = AlignedWavData {
    data: *include_bytes!("./sample.wav"),
};

pub struct WavStream {
    samples: &'static [i16],
    cursor: AtomicUsize,
}

impl WavStream {
    pub fn new() -> Self {
        let bytes = &RAW_WAV.data;
        let wav_format_size = core::mem::size_of::<WavFormatHeader>();
        let audio_bytes = &bytes[wav_format_size..];
        let (prefix, samples, suffix) = unsafe { audio_bytes.align_to::<i16>() };
        assert!(prefix.is_empty());
        assert!(suffix.is_empty());

        Self {
            samples,
            cursor: AtomicUsize::new(0),
        }
    }

    /* fn get_header(&self) -> WavFormatHeader {
        let header_bytes = &self.bytes[..self.header_size];
        let header = unsafe { &*(header_bytes.as_ptr() as *const WavFormatHeader) };
        header.clone()
    } */

    pub fn next_samples(&self, requested_frames: usize) -> &'static [i16] {
        let elements_needed = requested_frames * 2;
        let mut cursor = self.cursor.load(core::sync::atomic::Ordering::Relaxed);

        if cursor + elements_needed > self.samples.len() {
            cursor = 0;
        }

        let sliced_samples = &self.samples[cursor..(cursor + elements_needed)];
        self.cursor.store(
            cursor + elements_needed,
            core::sync::atomic::Ordering::Relaxed,
        );
        sliced_samples
    }
}

lazy_static::lazy_static! {
    pub static ref WAV: WavStream = WavStream::new();
}

pub fn setup_bg_stream() {
    let samples = WAV.next_samples(4096);
    let buffer_ptr = PCA_BUFFER_VIRT_START as *mut PcmSample;
    for (i, lr_samples) in samples.chunks_exact(2).enumerate() {
        let left = lr_samples[0];
        let right = lr_samples[1];

        unsafe {
            *buffer_ptr.add(i) = PcmSample::new().with_left(left).with_right(right);
        }
    }
}
