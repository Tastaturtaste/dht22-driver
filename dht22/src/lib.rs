use fugit::{MicrosDurationU32, TimerDurationU32, TimerInstantU32};
#[cfg(feature = "std")]
use thiserror::Error;

#[derive(Debug)]
#[cfg_attr(feature = "std", derive(Error))]
pub enum DhtError<DeviceError> {
    #[error("Handshake failed")]
    Handshake(u8),
    #[error("Timeout in data transmission after {0}")]
    DataTransmissionTimeout(MicrosDurationU32),
    #[error("Checksum validation failed")]
    Checksum,
    #[error("DeviceError: {0}")]
    DeviceError(#[from] DeviceError),
}

pub trait IOPin {
    type DeviceError;
    fn set_low(&mut self) -> Result<(), Self::DeviceError>;
    fn set_high(&mut self) -> Result<(), Self::DeviceError>;
    fn is_low(&self) -> bool;
    fn is_high(&self) -> bool;
}

pub trait MicroTimer<const FREQ_HZ: u32> {
    fn now(&self) -> TimerInstantU32<FREQ_HZ>;
}

pub struct Dht22<Pin, ClockT, const FREQ_HZ: u32>
where
    Pin: IOPin,
    ClockT: MicroTimer<FREQ_HZ>,
{
    pin: Pin,
    clock: ClockT,
}

pub struct SensorReading {
    pub humidity: f32,
    pub temperature: f32,
}

impl<Pin, ClockT, const FREQ_HZ: u32> Dht22<Pin, ClockT, FREQ_HZ>
where
    Pin: IOPin,
    <Pin as IOPin>::DeviceError: core::fmt::Debug,
    ClockT: MicroTimer<FREQ_HZ>,
{
    pub fn new(pin: Pin, clock: ClockT) -> Self {
        Self { pin, clock }
    }
    pub fn read(&mut self) -> Result<SensorReading, DhtError<Pin::DeviceError>> {
        const RESPONSE_BITS: usize = 40;
        let mut pin = scopeguard::guard(&mut self.pin, |pin| {
            pin.set_high()
                .expect("Failed to reset pin to high at scope exit of Dht22::read!");
        });
        // Each bit is indicated by the two edges of the HIGH level (up, down).
        // In addition the initial down edge from the get-ready HIGH state is recorded.
        let mut cycles: [TimerDurationU32<FREQ_HZ>; 2 * RESPONSE_BITS + 1] =
            [TimerDurationU32::from_ticks(0); 2 * RESPONSE_BITS + 1];
        let waiter = Waiter { clock: &self.clock };
        // Disable interrupts while interacting with the sensor so they don't mess up the timings
        let result = critical_section::with(|_guard| {
            // Initial handshake
            pin.set_low()?;
            let _ = waiter.wait_for(|| false, TimerDurationU32::micros(1200));
            pin.set_high()?;

            // Wait for DHT22 to acknowledge the handshake with low
            if waiter
                .wait_for(|| pin.is_low(), TimerDurationU32::micros(100))
                .is_err()
            {
                return Err(DhtError::Handshake(0));
            }

            // Wait for low to end
            if waiter
                .wait_for(|| pin.is_high(), TimerDurationU32::micros(100))
                .is_err()
            {
                return Err(DhtError::Handshake(1));
            }
            // Data transfer started. Each bit starts with 50us low and than ~27us high for a 0 or ~70us high for a 1.
            // The pin should stay high for about 80us before transmission starts, we don't actually care about the precise timing of this duration.
            // To be precise just record time of edges and process later.
            let mut is_high = true;
            for duration in &mut cycles {
                *duration = waiter
                    .wait_for(|| is_high != pin.is_high(), TimerDurationU32::micros(100))
                    .map_err(|duration| DhtError::DataTransmissionTimeout(duration.convert()))?;

                is_high = !is_high;
            }
            // Data transfer ended
            Ok(())
        });
        if result.is_err() {
            println!("{:#?}", cycles);
            result?;
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
            println!("Cycles: {cycles:#?}");
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

struct Waiter<'clock, ClockT, const FREQ_HZ: u32>
where
    ClockT: MicroTimer<FREQ_HZ>,
{
    clock: &'clock ClockT,
}
impl<'clock, ClockT, const FREQ_HZ: u32> Waiter<'clock, ClockT, FREQ_HZ>
where
    ClockT: MicroTimer<FREQ_HZ>,
{
    #[inline(always)]
    fn wait_for(
        &self,
        condition: impl Fn() -> bool,
        timeout: MicrosDurationU32,
    ) -> Result<TimerDurationU32<FREQ_HZ>, TimerDurationU32<FREQ_HZ>> {
        let start = self.clock.now();
        loop {
            let since_start = self.clock.now() - start;
            if condition() {
                return Ok(since_start);
            }
            if since_start > timeout {
                return Err(since_start);
            }
        }
    }
}
