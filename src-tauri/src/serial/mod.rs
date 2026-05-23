//! Serial port channel. Parameter shape mirrors Tabby v1 to keep the config
//! migration trivial.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialProfile {
    pub port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: StopBits,
    pub flow_control: FlowControl,
}

impl Default for SerialProfile {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: 115_200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow_control: FlowControl::None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Parity {
    None,
    Even,
    Odd,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum StopBits {
    One,
    Two,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FlowControl {
    None,
    Software,
    Hardware,
}

impl From<Parity> for serialport::Parity {
    fn from(p: Parity) -> Self {
        match p {
            Parity::None => serialport::Parity::None,
            Parity::Even => serialport::Parity::Even,
            Parity::Odd => serialport::Parity::Odd,
        }
    }
}

impl From<StopBits> for serialport::StopBits {
    fn from(s: StopBits) -> Self {
        match s {
            StopBits::One => serialport::StopBits::One,
            StopBits::Two => serialport::StopBits::Two,
        }
    }
}

impl From<FlowControl> for serialport::FlowControl {
    fn from(f: FlowControl) -> Self {
        match f {
            FlowControl::None => serialport::FlowControl::None,
            FlowControl::Software => serialport::FlowControl::Software,
            FlowControl::Hardware => serialport::FlowControl::Hardware,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SerialError {
    #[error("open: {0}")]
    Open(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("channel closed")]
    Closed,
}

pub struct SerialChannel {
    port: Arc<Mutex<Box<dyn serialport::SerialPort>>>,
    output_rx: Option<mpsc::Receiver<Vec<u8>>>,
}

impl SerialChannel {
    pub fn open(profile: &SerialProfile) -> Result<Self, SerialError> {
        let port = serialport::new(&profile.port, profile.baud)
            .data_bits(match profile.data_bits {
                5 => serialport::DataBits::Five,
                6 => serialport::DataBits::Six,
                7 => serialport::DataBits::Seven,
                _ => serialport::DataBits::Eight,
            })
            .parity(profile.parity.into())
            .stop_bits(profile.stop_bits.into())
            .flow_control(profile.flow_control.into())
            .timeout(std::time::Duration::from_millis(50))
            .open()
            .map_err(|e| SerialError::Open(e.to_string()))?;

        let mut reader = port
            .try_clone()
            .map_err(|e| SerialError::Open(e.to_string()))?;

        let (tx, rx) = mpsc::channel::<Vec<u8>>(64);
        std::thread::Builder::new()
            .name("serial-reader".into())
            .spawn(move || {
                let mut buf = [0u8; 2048];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => continue,
                        Ok(n) => {
                            if tx.blocking_send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => continue,
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
            .map_err(SerialError::Io)?;

        Ok(Self {
            port: Arc::new(Mutex::new(port)),
            output_rx: Some(rx),
        })
    }

    pub fn take_output(&mut self) -> Option<mpsc::Receiver<Vec<u8>>> {
        self.output_rx.take()
    }

    pub fn write(&self, data: &[u8]) -> Result<(), SerialError> {
        let mut p = self.port.lock().map_err(|_| SerialError::Closed)?;
        p.write_all(data)?;
        p.flush()?;
        Ok(())
    }
}

/// Lists available serial ports.
pub fn list_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default()
}

pub async fn init() -> crate::Result<()> {
    tracing::debug!("serial::init");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parity_mapping() {
        assert_eq!(
            serialport::Parity::from(Parity::Even),
            serialport::Parity::Even
        );
    }

    #[test]
    fn list_ports_does_not_panic() {
        let _ = list_ports();
    }

    #[test]
    fn open_missing_port_errors() {
        let prof = SerialProfile {
            port: "/dev/this-does-not-exist-tabby".into(),
            ..Default::default()
        };
        assert!(SerialChannel::open(&prof).is_err());
    }
}
