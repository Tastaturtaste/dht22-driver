use std::time::Duration;

use esp_idf_hal::{
    gpio::{IOPin, Level, PinDriver},
    interrupt::IsrCriticalSection,
    peripheral::Peripheral,
    timer::TimerDriver,
};
use esp_idf_svc::hal::{delay::Ets, timer};
use esp_idf_sys::EspError;
use thiserror::Error;

use crate::util::WithCleanup;

#[derive(Error, Debug)]
pub enum DhtError {
    #[error("Sensor is not initialized")]
    NotInitialized,
    #[error("Handshake failed")]
    Handshake(u8),
    #[error("Timeout in data protocol")]
    DataTransmission,
    #[error("Checksum validation failed")]
    Checksum,
    #[error("EspError: {0}")]
    EspError(#[from] EspError),
}

pub struct Dht22<Pin, Timer> {
    pin: Pin,
    timer: Timer,
}

pub struct SensorReading {
    pub humidity: f32,
    pub temperature: f32,
}

pub struct Waiter {
    pub(crate) timer: esp_idf_svc::timer::EspTaskTimerService,
}
impl Waiter {
    pub fn new() -> Self {
        Self {
            timer: esp_idf_svc::timer::EspTimerService::new().expect("Failed to initialize timer!"),
        }
    }

    pub fn wait_for(
        &self,
        condition: impl Fn() -> bool,
        timeout: Duration,
    ) -> Result<Duration, Duration> {
        let start = self.timer.now();
        while (self.timer.now() - start) < timeout {
            if condition() {
                return Ok(self.timer.now() - start);
            }
        }
        Err(self.timer.now() - start)
    }
}

impl<Pin: IOPin, Timer> Dht22<Pin, Timer>
where
    Timer: timer::Timer,
    // Timer: std::ops::DerefMut,
    // <Timer as std::ops::Deref>::Target: Peripheral,
    // <<Timer as std::ops::Deref>::Target as esp_idf_hal::peripheral::Peripheral>::P: timer::Timer,
{
    pub fn new(pin: Pin, timer: Timer) -> Self {
        Self { pin, timer }
    }
    pub fn read(&mut self) -> Result<SensorReading, DhtError> {
        let mut pin = WithCleanup::new(PinDriver::input_output_od(&mut self.pin)?, |mut p| {
            p.set_high()
                .expect("Failed to reset pin to high after reading the sensor DHT22");
        });
        let timer_config = timer::config::Config::new();
        // let mut timer = TimerDriver::new(&mut self.timer, &timer_config)?;

        // Prepare Waiter
        let waiter = Waiter::new();
        // Disable interrupts so they don't mess up the timings
        let section = IsrCriticalSection::new();
        let _section_guard = section.enter();
        // Initial handshake
        pin.set_low()?;
        Ets::delay_us(5000);
        pin.set_high()?;

        // Wait for DHT22 to acknowledge the handshake with low
        if waiter
            .wait_for(|| pin.is_set_low(), Duration::from_micros(100))
            .is_err()
        {
            return Err(DhtError::Handshake(0));
        }

        // Wait for low to end
        if waiter
            .wait_for(|| pin.is_set_high(), Duration::from_micros(90))
            .is_err()
        {
            return Err(DhtError::Handshake(1));
        }
        // The pin should stay high for about 80us before transmission starts.
        // The edge from high to low indicates the start of transmission and the following low period is already part of the transmission.
        if waiter
            .wait_for(|| pin.is_set_low(), Duration::from_micros(90))
            .is_err()
        {
            return Err(DhtError::Handshake(2));
        }
        // Data transfer started. Each bit starts with 50us low and than ~27us high for a 0 or ~70us high for a 1.
        const RESPONSE_BITS: usize = 40;
        let mut cycles: [Duration; 2 * RESPONSE_BITS] = [Default::default(); 2 * RESPONSE_BITS];
        for i in 0..RESPONSE_BITS {
            unsafe {
                *cycles.get_unchecked_mut(2 * i) = waiter
                    .wait_for(|| pin.is_set_high(), Duration::from_micros(70))
                    .map_err(|_| DhtError::DataTransmission)?;
                *cycles.get_unchecked_mut(2 * i + 1) = waiter
                    .wait_for(|| pin.is_set_low(), Duration::from_micros(100))
                    .map_err(|_| DhtError::DataTransmission)?;
            }
        }
        let mut bytes: [u8; 5] = [0; 5];
        for (bit_idx, _) in cycles
            .chunks_exact(2)
            .map(|pair| {
                let cycles_low = pair[0];
                let cycles_high = pair[1];
                cycles_low < cycles_high
            })
            .enumerate()
            .filter(|(_, bit)| *bit)
        {
            let byte_idx = bit_idx / 8;
            bytes[byte_idx] |= 1 << (7 - bit_idx);
        }
        // Verify the checksum in the last byte
        if bytes[0]
            .wrapping_add(bytes[1])
            .wrapping_add(bytes[2])
            .wrapping_add(bytes[3])
            != bytes[4]
        {
            return Err(DhtError::Checksum);
        }
        let humidity = (((bytes[0] as u32) << 8 | bytes[1] as u32) / 10) as f32;
        let temperature = (((bytes[2] as u32) << 8 | bytes[3] as u32) / 10) as f32;
        Ok(SensorReading {
            humidity,
            temperature,
        })
    }
}
