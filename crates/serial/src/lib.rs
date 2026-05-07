use crossbeam::channel::{self, Receiver};
use newvofa_protocol::{Frame, ParameterEntry, UnifiedParser};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum SerialMessage {
    Frame(Vec<f32>),
    Parameter(Vec<ParameterEntry>),
    RawData(Vec<u8>),
}

pub struct PortInfo {
    pub name: String,
    pub description: String,
}

pub struct SerialConfig {
    pub port_name: String,
    pub baud_rate: u32,
    pub data_bits: DataBits,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow_control: FlowControl,
    pub timeout_ms: u64,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            port_name: String::new(),
            baud_rate: 115200,
            data_bits: DataBits::Eight,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow_control: FlowControl::None,
            timeout_ms: 100,
        }
    }
}

pub struct SerialManager {
    port: Option<Box<dyn serialport::SerialPort>>,
    running: Arc<AtomicBool>,
    read_thread: Option<thread::JoinHandle<()>>,
}

impl SerialManager {
    pub fn list_ports() -> Vec<PortInfo> {
        serialport::available_ports()
            .unwrap_or_default()
            .into_iter()
            .map(|p| PortInfo {
                name: p.port_name.clone(),
                description: format!("{:?}", p.port_type),
            })
            .collect()
    }

    pub fn open(config: SerialConfig) -> Result<(Self, Receiver<SerialMessage>), String> {
        let port = serialport::new(&config.port_name, config.baud_rate)
            .data_bits(config.data_bits)
            .parity(config.parity)
            .stop_bits(config.stop_bits)
            .flow_control(config.flow_control)
            .timeout(Duration::from_millis(config.timeout_ms))
            .open()
            .map_err(|e| format!("Failed to open {}: {}", config.port_name, e))?;

        let running = Arc::new(AtomicBool::new(true));
        let running_clone = Arc::clone(&running);

        let (tx, rx) = channel::unbounded::<SerialMessage>();

        let mut port_clone = port.try_clone().map_err(|e| format!("Failed to clone port: {}", e))?;

        let read_thread = thread::Builder::new()
            .name("serial-reader".into())
            .spawn(move || {
                let mut parser = UnifiedParser::new();
                let mut buf = [0u8; 1024];

                while running_clone.load(Ordering::Relaxed) {
                    match port_clone.read(&mut buf) {
                        Ok(0) => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Ok(n) => {
                            let raw = buf[..n].to_vec();
                            if tx.send(SerialMessage::RawData(raw)).is_err() {
                                return;
                            }
                            let frames = parser.feed_bytes(&buf[..n]);
                            for frame in frames {
                                match frame {
                                    Frame::JustFloat(data) => {
                                        if tx.send(SerialMessage::Frame(data)).is_err() {
                                            return;
                                        }
                                    }
                                    Frame::Parameter(entries) => {
                                        if tx.send(SerialMessage::Parameter(entries)).is_err() {
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                            continue;
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
            })
            .map_err(|e| format!("Failed to spawn read thread: {}", e))?;

        Ok((
            Self {
                port: Some(port),
                running,
                read_thread: Some(read_thread),
            },
            rx,
        ))
    }

    pub fn send_command(&mut self, name: &str, value: f32) -> Result<(), String> {
        let cmd = format!("{}:{}\r\n", name, value);
        if let Some(ref mut port) = self.port {
            port.write_all(cmd.as_bytes())
                .map_err(|e| format!("Failed to send command: {}", e))?;
            port.flush()
                .map_err(|e| format!("Failed to flush: {}", e))?;
        }
        Ok(())
    }

    pub fn is_open(&self) -> bool {
        self.port.is_some()
    }
}

impl Drop for SerialManager {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.read_thread.take() {
            let _ = handle.join();
        }
    }
}
