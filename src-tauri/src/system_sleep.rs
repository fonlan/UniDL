use std::{sync::mpsc, thread};

struct SleepRequest {
    prevent_sleep: bool,
    result_sender: Option<mpsc::Sender<Result<(), String>>>,
}

pub struct SleepState {
    sender: mpsc::Sender<SleepRequest>,
    prevent_sleep: bool,
}

impl SleepState {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel::<SleepRequest>();
        thread::spawn(move || {
            let mut current = false;
            while let Ok(request) = receiver.recv() {
                if current == request.prevent_sleep {
                    if let Some(result_sender) = request.result_sender {
                        let _ = result_sender.send(Ok(()));
                    }
                    continue;
                }
                let result = apply_prevent_sleep(request.prevent_sleep);
                if result.is_ok() {
                    current = request.prevent_sleep;
                }
                if let Some(result_sender) = request.result_sender {
                    let _ = result_sender.send(result);
                }
            }
            if current {
                let _ = apply_prevent_sleep(false);
            }
        });

        Self {
            sender,
            prevent_sleep: false,
        }
    }

    pub fn set_prevent_sleep(&mut self, prevent_sleep: bool) -> Result<(), String> {
        if self.prevent_sleep == prevent_sleep {
            return Ok(());
        }

        let (result_sender, result_receiver) = mpsc::channel();
        self.sender
            .send(SleepRequest {
                prevent_sleep,
                result_sender: Some(result_sender),
            })
            .map_err(|_| "sleep prevention worker is not available".to_string())?;
        result_receiver
            .recv()
            .map_err(|_| "sleep prevention worker stopped unexpectedly".to_string())??;
        self.prevent_sleep = prevent_sleep;
        Ok(())
    }
}

impl Drop for SleepState {
    fn drop(&mut self) {
        if self.prevent_sleep {
            let _ = self.sender.send(SleepRequest {
                prevent_sleep: false,
                result_sender: None,
            });
        }
    }
}

#[cfg(windows)]
fn apply_prevent_sleep(prevent_sleep: bool) -> Result<(), String> {
    use windows_sys::Win32::System::Power::{
        SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
    };

    let flags = if prevent_sleep {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    let previous = unsafe { SetThreadExecutionState(flags) };
    if previous == 0 {
        return Err("failed to update Windows sleep prevention state".to_string());
    }
    Ok(())
}

#[cfg(not(windows))]
fn apply_prevent_sleep(prevent_sleep: bool) -> Result<(), String> {
    if prevent_sleep {
        return Err("preventing system sleep is only supported on Windows".to_string());
    }
    Ok(())
}
