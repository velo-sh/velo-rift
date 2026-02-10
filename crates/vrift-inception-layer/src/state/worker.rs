// =============================================================================
// state/worker.rs — Background worker thread
// =============================================================================
//
// Manages the background worker thread that processes tasks from the ring buffer.
// Includes:
//   - spawn_worker()  — #[inline(never)] to isolate pthread_create side effects
//   - worker_entry()  — adaptive backoff loop (spin → yield → sleep)
//   - process_task()  — dispatch ring buffer tasks
// =============================================================================

use std::sync::atomic::Ordering;

use super::{InceptionLayerState, DIRTY_TRACKER, WORKER_STARTED};

impl InceptionLayerState {
    /// BUG-007b: Must not inline — pthread_create internally calls mmap (interposed).
    /// Keeps get()'s stack frame small and isolates pthread_create side effects.
    #[inline(never)]
    pub(super) fn spawn_worker() {
        // Double-check to ensure we don't spawn multiple times racefully
        if WORKER_STARTED.swap(true, Ordering::SeqCst) {
            return;
        }

        unsafe {
            let mut thread: libc::pthread_t = std::mem::zeroed();
            libc::pthread_create(
                &mut thread,
                std::ptr::null(),
                Self::worker_entry,
                std::ptr::null_mut(),
            );
            libc::pthread_detach(thread);
        }
    }

    extern "C" fn worker_entry(_: *mut libc::c_void) -> *mut libc::c_void {
        // Block all signals in worker thread
        unsafe {
            let mut mask: libc::sigset_t = std::mem::zeroed();
            libc::sigfillset(&mut mask);
            libc::pthread_sigmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut());
        }

        let reactor = match crate::sync::get_reactor() {
            Some(r) => r,
            None => return std::ptr::null_mut(),
        };

        // Worker thread loop with adaptive backoff for CPU efficiency
        let mut backoff_count = 0u32;
        loop {
            if let Some(task) = reactor.ring_buffer.pop() {
                // Reset backoff on success
                backoff_count = 0;
                Self::process_task(task);
            } else {
                // No task available - adaptive backoff
                backoff_count = backoff_count.saturating_add(1).min(1000);

                if backoff_count < 10 {
                    // Fast spin for very short idle periods
                    std::hint::spin_loop();
                } else if backoff_count < 100 {
                    // Yield CPU for short idle periods
                    std::thread::yield_now();
                } else {
                    // Sleep for prolonged idle (1μs reduces CPU while staying responsive)
                    std::thread::sleep(std::time::Duration::from_micros(1));
                }
            }
        }
    }

    fn process_task(task: crate::sync::Task) {
        match task {
            crate::sync::Task::ReclaimFd(_fd, entry) => {
                if !entry.is_null() {
                    let e = unsafe { Box::from_raw(entry) };
                    if e.lock_fd >= 0 {
                        unsafe { libc::close(e.lock_fd) };
                    }
                }
            }
            crate::sync::Task::Reingest { vpath, temp_path } => {
                if let Some(state) = InceptionLayerState::get_no_spawn() {
                    // Route reingest to vDird socket (not main daemon, which rejects it)
                    let socket = if !state.vdird_socket_path.is_empty() {
                        &state.vdird_socket_path
                    } else {
                        &state.socket_path
                    };
                    unsafe {
                        if crate::ipc::sync_ipc_manifest_reingest(socket, &vpath, &temp_path) {
                            // M4: Clear dirty status ONLY after the daemon confirms reingest.
                            DIRTY_TRACKER.clear_dirty(&vpath);
                        }
                    }
                }
            }
            crate::sync::Task::Log(msg) => {
                unsafe { libc::write(2, msg.as_ptr() as *const _, msg.len()) };
            }
            crate::sync::Task::IpcFireAndForget {
                socket_path,
                payload,
            } => {
                // Phase 3: Worker-side fire-and-forget IPC
                // Connect, register workspace, send pre-serialized payload
                unsafe {
                    crate::ipc::send_fire_and_forget_sync(&socket_path, &payload);
                }
            }
        }
    }
}
