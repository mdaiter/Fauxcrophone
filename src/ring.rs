//! Shared-memory friendly single-producer/single-consumer ring buffer.
use std::cell::UnsafeCell;
use std::mem::size_of;
use std::sync::atomic::{AtomicU64, Ordering};

use memmap2::{MmapMut, MmapOptions};
use once_cell::sync::Lazy;

#[cfg(target_os = "macos")]
use mach::mach_time::{mach_absolute_time, mach_timebase_info, mach_timebase_info_data_t};

/// Header stored at the front of a shared memory buffer so that peer processes can
/// inspect queue state without invoking Rust code.
#[repr(C, align(64))]
pub struct RingBufferHeader {
    capacity_frames: u32,
    channels: u32,
    reserved: u32,
    write_index: AtomicU64,
    read_index: AtomicU64,
    last_timestamp_ns: AtomicU64,
}

impl RingBufferHeader {
    fn new(capacity_frames: usize, channels: usize) -> Self {
        Self {
            capacity_frames: capacity_frames as u32,
            channels: channels as u32,
            reserved: 0,
            write_index: AtomicU64::new(0),
            read_index: AtomicU64::new(0),
            last_timestamp_ns: AtomicU64::new(0),
        }
    }

    fn capacity_frames(&self) -> usize {
        self.capacity_frames as usize
    }

    fn channels(&self) -> usize {
        self.channels as usize
    }
}

enum RingStorage {
    Local {
        header: UnsafeCell<RingBufferHeader>,
        data: UnsafeCell<Vec<f32>>,
    },
    Shared {
        mmap: UnsafeCell<MmapMut>,
        header_ptr: *mut RingBufferHeader,
        data_ptr: *mut f32,
    },
}

unsafe impl Send for RingStorage {}
unsafe impl Sync for RingStorage {}

/// Lock-free ring buffer for interleaved `f32` audio data.
pub struct SharedRingBuffer {
    storage: RingStorage,
    capacity_frames: usize,
    channels: usize,
}

unsafe impl Send for SharedRingBuffer {}
unsafe impl Sync for SharedRingBuffer {}

impl SharedRingBuffer {
    /// Create a local ring buffer. The storage is still shareable across FFI callers
    /// through `raw_header_ptr`/`raw_data_ptr`.
    pub fn new_local(capacity_frames: usize, channels: usize) -> Self {
        let data_len = capacity_frames * channels;
        let data = vec![0.0f32; data_len];
        Self {
            storage: RingStorage::Local {
                header: UnsafeCell::new(RingBufferHeader::new(capacity_frames, channels)),
                data: UnsafeCell::new(data),
            },
            capacity_frames,
            channels,
        }
    }

    /// Create an anonymous shared memory backed ring buffer using `mmap`.
    pub fn new_shared(capacity_frames: usize, channels: usize) -> std::io::Result<Self> {
        let samples = capacity_frames * channels;
        let bytes = size_of::<RingBufferHeader>() + size_of::<f32>() * samples;
        let mut mmap = MmapOptions::new().len(bytes).map_anon()?;
        {
            let header_ptr = mmap.as_mut_ptr() as *mut RingBufferHeader;
            unsafe {
                header_ptr.write(RingBufferHeader::new(capacity_frames, channels));
            }
        }
        let header_ptr = mmap.as_mut_ptr() as *mut RingBufferHeader;
        let data_ptr = unsafe { mmap.as_mut_ptr().add(size_of::<RingBufferHeader>()) as *mut f32 };
        Ok(Self {
            storage: RingStorage::Shared {
                mmap: UnsafeCell::new(mmap),
                header_ptr,
                data_ptr,
            },
            capacity_frames,
            channels,
        })
    }

    /// Create from an existing `MmapMut` region that follows the header+data layout.
    pub fn from_mmap(mut mmap: MmapMut, channels: usize) -> Self {
        let header_ptr = mmap.as_mut_ptr() as *mut RingBufferHeader;
        let capacity_frames = unsafe { (*header_ptr).capacity_frames() };
        debug_assert_eq!(
            channels,
            unsafe { (*header_ptr).channels() },
            "channel mismatch between header and constructor"
        );
        let data_ptr = unsafe { mmap.as_mut_ptr().add(size_of::<RingBufferHeader>()) as *mut f32 };
        Self {
            storage: RingStorage::Shared {
                mmap: UnsafeCell::new(mmap),
                header_ptr,
                data_ptr,
            },
            capacity_frames,
            channels,
        }
    }

    fn header(&self) -> &RingBufferHeader {
        match &self.storage {
            RingStorage::Local { header, .. } => unsafe { &*header.get() },
            RingStorage::Shared { header_ptr, .. } => unsafe { &**header_ptr },
        }
    }

    fn header_mut(&self) -> &RingBufferHeader {
        self.header()
    }

    fn data_slice(&self) -> &[f32] {
        match &self.storage {
            RingStorage::Local { data, .. } => unsafe {
                let vec_ref: &Vec<f32> = &*data.get();
                &vec_ref[..]
            },
            RingStorage::Shared { mmap, data_ptr, .. } => {
                let mmap = unsafe { &*mmap.get() };
                let samples = self.capacity_frames * self.channels;
                let data_bytes = mmap.len().saturating_sub(size_of::<RingBufferHeader>());
                let available = data_bytes / size_of::<f32>();
                unsafe { std::slice::from_raw_parts(*data_ptr, samples.min(available)) }
            }
        }
    }

    fn data_slice_mut(&self) -> &mut [f32] {
        match &self.storage {
            RingStorage::Local { data, .. } => unsafe {
                let vec_mut: &mut Vec<f32> = &mut *data.get();
                &mut vec_mut[..]
            },
            RingStorage::Shared { mmap, data_ptr, .. } => {
                let mmap = unsafe { &mut *mmap.get() };
                let samples = self.capacity_frames * self.channels;
                let data_bytes = mmap.len().saturating_sub(size_of::<RingBufferHeader>());
                let available = data_bytes / size_of::<f32>();
                unsafe { std::slice::from_raw_parts_mut(*data_ptr, samples.min(available)) }
            }
        }
    }

    /// Total capacity in frames.
    pub fn capacity_frames(&self) -> usize {
        self.capacity_frames
    }

    /// Total capacity in samples (frames * channels).
    pub fn capacity_samples(&self) -> usize {
        self.capacity_frames * self.channels
    }

    /// Pointer to the shared header.
    pub fn raw_header_ptr(&self) -> *mut RingBufferHeader {
        match &self.storage {
            RingStorage::Local { header, .. } => header.get(),
            RingStorage::Shared { header_ptr, .. } => *header_ptr,
        }
    }

    /// Pointer to the interleaved sample data region.
    pub fn raw_data_ptr(&self) -> *mut f32 {
        match &self.storage {
            RingStorage::Local { data, .. } => unsafe { (*data.get()).as_mut_ptr() },
            RingStorage::Shared { data_ptr, .. } => *data_ptr,
        }
    }

    /// Push frames into the ring, returning frames written.
    pub fn push(&self, frames: &[f32], timestamp_ns: Option<u64>) -> usize {
        let header = self.header_mut();
        let frames_count = frames.len() / self.channels;
        if frames_count == 0 {
            return 0;
        }

        let capacity = self.capacity_frames as u64;
        let write_index = header.write_index.load(Ordering::Acquire);
        let read_index = header.read_index.load(Ordering::Acquire);
        let used = write_index.saturating_sub(read_index).min(capacity);
        let free = capacity.saturating_sub(used);
        if free == 0 {
            return 0;
        }
        let frames_to_write = frames_count.min(free as usize);
        let mut src_offset = 0;
        let data = self.data_slice_mut();

        let start_frame = (write_index % capacity) as usize;
        let first_chunk_frames = (self.capacity_frames - start_frame).min(frames_to_write);
        let first_samples = first_chunk_frames * self.channels;
        let first_dest = start_frame * self.channels;
        data[first_dest..first_dest + first_samples].copy_from_slice(&frames[..first_samples]);
        src_offset += first_samples;

        if frames_to_write > first_chunk_frames {
            let remaining_frames = frames_to_write - first_chunk_frames;
            let remaining_samples = remaining_frames * self.channels;
            data[0..remaining_samples]
                .copy_from_slice(&frames[src_offset..src_offset + remaining_samples]);
        }

        let new_write = write_index + frames_to_write as u64;
        header.write_index.store(new_write, Ordering::Release);
        let timestamp = timestamp_ns.unwrap_or_else(monotonic_timestamp_ns);
        header.last_timestamp_ns.store(timestamp, Ordering::Release);
        frames_to_write
    }

    /// Pop frames into the provided buffer, returning frames read.
    pub fn pop(&self, out: &mut [f32]) -> usize {
        let header = self.header_mut();
        let requested_frames = out.len() / self.channels;
        if requested_frames == 0 {
            return 0;
        }
        let capacity = self.capacity_frames as u64;
        let write_index = header.write_index.load(Ordering::Acquire);
        let read_index = header.read_index.load(Ordering::Acquire);
        let available = write_index.saturating_sub(read_index).min(capacity);
        if available == 0 {
            return 0;
        }
        let frames_to_read = requested_frames.min(available as usize);
        let mut samples_copied = 0usize;
        let data = self.data_slice();

        let start_frame = (read_index % capacity) as usize;
        let first_chunk_frames = (self.capacity_frames - start_frame).min(frames_to_read);
        let first_samples = first_chunk_frames * self.channels;
        let first_src = start_frame * self.channels;
        out[..first_samples].copy_from_slice(&data[first_src..first_src + first_samples]);
        samples_copied += first_samples;

        if frames_to_read > first_chunk_frames {
            let remaining_frames = frames_to_read - first_chunk_frames;
            let remaining_samples = remaining_frames * self.channels;
            out[samples_copied..samples_copied + remaining_samples]
                .copy_from_slice(&data[0..remaining_samples]);
        }

        header
            .read_index
            .store(read_index + frames_to_read as u64, Ordering::Release);
        frames_to_read
    }

    /// Drop frames without copying, returning the number discarded.
    pub fn discard(&self, frames: usize) -> usize {
        let header = self.header_mut();
        let capacity = self.capacity_frames as u64;
        let write_index = header.write_index.load(Ordering::Acquire);
        let read_index = header.read_index.load(Ordering::Acquire);
        let available = write_index.saturating_sub(read_index).min(capacity);
        if available == 0 {
            return 0;
        }
        let frames = frames.min(available as usize);
        header
            .read_index
            .store(read_index + frames as u64, Ordering::Release);
        frames
    }

    /// Frames ready for reading.
    pub fn available_read(&self) -> usize {
        let header = self.header();
        let capacity = self.capacity_frames as u64;
        let write_index = header.write_index.load(Ordering::Acquire);
        let read_index = header.read_index.load(Ordering::Acquire);
        write_index.saturating_sub(read_index).min(capacity) as usize
    }

    /// Timestamp of the last write.
    pub fn last_timestamp_ns(&self) -> u64 {
        self.header().last_timestamp_ns.load(Ordering::Acquire)
    }
}

#[cfg(target_os = "macos")]
fn timebase() -> (u64, u64) {
    static TIMEBASE: Lazy<(u64, u64)> = Lazy::new(|| unsafe {
        let mut info = mach_timebase_info_data_t::default();
        mach_timebase_info(&mut info);
        (info.numer as u64, info.denom as u64)
    });
    *TIMEBASE
}

/// Convert a mach host time tick count into nanoseconds.
pub fn host_time_to_ns(host_time: u64) -> u64 {
    #[cfg(target_os = "macos")]
    {
        if host_time == 0 {
            return 0;
        }
        let (numer, denom) = timebase();
        ((host_time as u128 * numer as u128) / denom as u128) as u64
    }
    #[cfg(not(target_os = "macos"))]
    {
        host_time
    }
}

/// Monotonic timestamp in nanoseconds.
pub fn monotonic_timestamp_ns() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let host_time = unsafe { mach_absolute_time() };
        host_time_to_ns(host_time)
    }
    #[cfg(not(target_os = "macos"))]
    {
        static START: Lazy<(std::time::Instant, u64)> = Lazy::new(|| {
            let instant = std::time::Instant::now();
            (instant, 0)
        });
        let elapsed = START.0.elapsed();
        (elapsed.as_secs() * 1_000_000_000) + elapsed.subsec_nanos() as u64
    }
}
