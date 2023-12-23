use std::time::Duration;

use esp_idf_hal::{
    delay::Ets,
    gpio::{IOPin, PinDriver},
    interrupt::IsrCriticalSection,
};
use esp_idf_sys::EspError;
use thiserror::Error;

use crate::util::{Waiter, WithCleanup};

#[derive(Error, Debug)]
pub enum DhtError {
    #[error("Handshake failed")]
    Handshake(u8),
    #[error("Timeout in data protocol")]
    DataTransmission,
    #[error("Checksum validation failed")]
    Checksum,
    #[error("EspError: {0}")]
    EspError(#[from] EspError),
}

pub struct Dht22<Pin> {
    pin: Pin,
    waiter: Waiter,
}

pub struct SensorReading {
    pub humidity: f32,
    pub temperature: f32,
}

impl<Pin: IOPin> Dht22<Pin> {
    pub fn new(pin: Pin) -> Self {
        Self {
            pin,
            waiter: Waiter::new(),
        }
    }
    pub fn read(&mut self) -> Result<SensorReading, DhtError> {
        // Make sure the pin is reset to idle state when finished
        let mut pin = WithCleanup::new(PinDriver::input_output_od(&mut self.pin)?, |mut p| {
            p.set_high()
                .expect("Failed to reset pin to high after reading the sensor DHT22");
        });

        const RESPONSE_BITS: usize = 40;
        // Each bit is indicated by the two edges of the HIGH level (up, down).
        // In addition the initial down edge from the get-ready HIGH state is recorded.
        let mut cycles: [Duration; 2 * RESPONSE_BITS + 1] =
            [Default::default(); 2 * RESPONSE_BITS + 1];
        {
            // Disable interrupts so they don't mess up the timings
            let section = IsrCriticalSection::new();
            let _section_guard = section.enter();
            // Initial handshake

            pin.set_low()?;
            Ets::delay_us(1200);
            pin.set_high()?;

            // Wait for DHT22 to acknowledge the handshake with low
            if self
                .waiter
                .wait_for(|| pin.is_low(), Duration::from_micros(100))
                .is_err()
            {
                return Err(DhtError::Handshake(0));
            }

            // Wait for low to end
            if self
                .waiter
                .wait_for(|| pin.is_high(), Duration::from_micros(90))
                .is_err()
            {
                return Err(DhtError::Handshake(1));
            }
            // Data transfer started. Each bit starts with 50us low and than ~27us high for a 0 or ~70us high for a 1.
            // The pin should stay high for about 80us before transmission starts, we don't actually care about the precise timing of this duration.
            // To be precise just record time of edges and process later.
            let mut is_low = false;
            for duration in &mut cycles {
                *duration = self
                    .waiter
                    .wait_for(|| is_low != pin.is_low(), Duration::from_micros(100))
                    .map_err(|_| DhtError::DataTransmission)?;
                is_low = !is_low;
            }
            // Data transfer ended
        }
        // Process the data
        for i in (1..cycles.len()).rev() {
            cycles[i] -= cycles[i - 1];
        }

        let mut bytes: [u8; 5] = [0; 5];
        // Ignore first element, because the time until data transmission starts is not important
        for (idx, _) in cycles[1..]
            // Group the durations of the low and high voltage for each bit
            .chunks_exact(2)
            // Map the duration of the high voltage to a 0 or 1
            .map(|pair| {
                let cycles_low = pair[0];
                let cycles_high = pair[1];
                // use the low duration as a reference to be robust against jitter
                cycles_low < cycles_high
            })
            // Count with index to know where to shift the bit
            .enumerate()
            // Ignore 0-bits as that is already their initial value
            .filter(|(_, bit)| *bit)
        {
            let byte_idx = idx / 8;
            let bit_idx = idx % 8;
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
