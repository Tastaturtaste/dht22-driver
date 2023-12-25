#![cfg_attr(not(feature = "std"), no_std)]
/// Example usage:
/// ```no_run
/// use dht22::IOPin;
/// // Newtype over device specific GPIO pin type
/// struct Pin;
/// #[derive(Debug)]
/// struct SomeMCUSpecificError;
/// impl IOPin for Pin
/// {
///     type DeviceError = SomeMCUSpecificError;
///     fn set_low(&mut self) -> Result<(), Self::DeviceError> {
///         todo!()
///     }
///
///     fn set_high(&mut self) -> Result<(), Self::DeviceError> {
///         todo!()
///     }
///
///     fn is_low(&self) -> bool {
///         todo!()
///     }
///
///     fn is_high(&self) -> bool {
///         todo!()
///     }
/// }
/// // Newtype over device specific clock/timer
/// struct MicroTimer;
///
/// // The timer uses the APB_CLK which typically ticks with 80 MHz https://docs.espressif.com/projects/esp-idf/en/latest/esp32c3/api-reference/system/system_time.html
/// // and per default uses a divider of 80. Therefor the timer tick frequency is 1MHz.
/// impl dht22::MicroTimer for MicroTimer {
///     fn now(&self) -> dht22::Microseconds {
///         dht22::Microseconds(
///             todo!()
///         )
///     }
/// }
/// fn main() -> Result<(), SomeMCUSpecificError> {
/// let mut pin = Pin;
/// pin.set_high()?;
/// let clock = MicroTimer;
/// let mut sensor = dht22::Dht22::new(pin, clock);
/// loop {
///     std::thread::sleep(std::time::Duration::from_secs(2));
///     if let Ok(reading) = sensor.read() {
///         println!("Humidity: {:?}, Temperature: {:?}", reading.humidity, reading.temperature);    
///     }
/// }
/// Ok(())
/// }
/// ```

#[cfg(feature = "std")]
use thiserror::Error;

#[derive(Debug)]
#[cfg_attr(feature = "std", derive(Error))]
pub enum DhtError<DeviceError> {
    #[cfg_attr(feature = "std", error("Handshake failed"))]
    Handshake(u8),
    #[cfg_attr(feature = "std", error("Timeout in data transmission after {}us", 0.0))]
    DataTransmissionTimeout(Microseconds),
    #[cfg_attr(feature = "std", error("Checksum validation failed"))]
    Checksum,
    #[cfg_attr(feature = "std", error("DeviceError: {0}"))]
    DeviceError(#[cfg_attr(feature = "std", from)] DeviceError),
}

/// Represents a GPIO pin capable of reading and setting the voltage level
pub trait IOPin {
    type DeviceError;
    fn set_low(&mut self) -> Result<(), Self::DeviceError>;
    fn set_high(&mut self) -> Result<(), Self::DeviceError>;
    fn is_low(&self) -> bool;
    fn is_high(&self) -> bool;
}

/// Simple Newtype to attach meaning to the contained primitive.
/// Represents a number of microseconds
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Microseconds(pub u32);

/// Represents a timer with microsecond resolution
pub trait MicroTimer {
    /// Return an instance in time as a number of microseconds since some fixed point (normally boot or start of the timer).
    /// The implementation is allowed to wrap during a read from the sensor.
    fn now(&self) -> Microseconds;
}

/// Represents a DHT22 sensor connected to a pin.
pub struct Dht22<Pin, ClockT>
where
    Pin: IOPin,
    ClockT: MicroTimer,
{
    pin: Pin,
    clock: ClockT,
}

/// A valid reading from the DHT22 sensor
pub struct SensorReading {
    pub humidity: f32,
    pub temperature: f32,
}

impl<Pin, ClockT> Dht22<Pin, ClockT>
where
    Pin: IOPin,
    <Pin as IOPin>::DeviceError: core::fmt::Debug,
    ClockT: MicroTimer,
{
    /// Construct a new representation of the DHT22 sensor.
    /// Construction is cheap as long as the pin and clock are cheap to move.
    pub fn new(pin: Pin, clock: ClockT) -> Self {
        Self { pin, clock }
    }
    /// Attempt one read from the DHT22 sensor.
    /// Between subsequent reads from the same sensor at least 2 seconds should pass to avoid erratic readings.
    /// Reading to early after startup may also result in failure to read.
    pub fn read(&mut self) -> Result<SensorReading, DhtError<Pin::DeviceError>> {
        const RESPONSE_BITS: usize = 40;
        let mut pin = scopeguard::guard(&mut self.pin, |pin| {
            pin.set_high()
                .expect("Failed to reset pin to high at scope exit of Dht22::read!");
        });
        // Each bit is indicated by the two edges of the HIGH level (up, down).
        // In addition the initial down edge from the get-ready HIGH state is recorded.
        let mut cycles: [u32; 2 * RESPONSE_BITS + 1] = [0; 2 * RESPONSE_BITS + 1];
        let waiter = Waiter { clock: &self.clock };
        // Disable interrupts while interacting with the sensor so they don't mess up the timings
        critical_section::with(|_guard| {
            // Initial handshake
            pin.set_low().map_err(DhtError::DeviceError)?;
            let _ = waiter.wait_for(|| false, 1200);
            pin.set_high().map_err(DhtError::DeviceError)?;

            // Wait for DHT22 to acknowledge the handshake with low
            if waiter.wait_for(|| pin.is_low(), 100).is_err() {
                return Err(DhtError::Handshake(0));
            }

            // Wait for low to end
            if waiter.wait_for(|| pin.is_high(), 100).is_err() {
                return Err(DhtError::Handshake(1));
            }
            // Data transfer started. Each bit starts with 50us low and than ~27us high for a 0 or ~70us high for a 1.
            // The pin should stay high for about 80us before transmission starts, we don't actually care about the precise timing of this duration.
            // To be precise just record time of edges and process later.
            let mut is_high = true;
            for duration in &mut cycles {
                *duration = waiter
                    .wait_for(|| is_high != pin.is_high(), 100)
                    .map(|microsecond| microsecond.0)
                    .map_err(DhtError::DataTransmissionTimeout)?;

                is_high = !is_high;
            }
            // Data transfer ended
            Ok(())
        })?;

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
        let humidity = (((bytes[0] as u32) << 8 | bytes[1] as u32) as f32) / 10.;
        // The MSB of the 16 temperature bits indicates negative temperatures
        let is_negative = (bytes[2] >> 7) != 0;
        bytes[2] &= 0b0111_1111;
        let temperature = (((bytes[2] as u32) << 8 | bytes[3] as u32) as f32) / 10.;
        let temperature = if is_negative {
            -1. * temperature
        } else {
            temperature
        };
        Ok(SensorReading {
            humidity,
            temperature,
        })
    }
}

struct Waiter<'clock, ClockT>
where
    ClockT: MicroTimer,
{
    clock: &'clock ClockT,
}
impl<'clock, ClockT> Waiter<'clock, ClockT>
where
    ClockT: MicroTimer,
{
    #[inline(always)]
    fn wait_for(
        &self,
        condition: impl Fn() -> bool,
        timeout: u32,
    ) -> Result<Microseconds, Microseconds> {
        let start = self.clock.now();
        loop {
            // Using wrapping arithmetic on unsigned integers, overflow of the timer can be
            // exploited to count over the whole representable range of the integer type regardless of initial value.
            // For example for a u8:
            // 10 - 230 = 36
            let since_start = self.clock.now().0.wrapping_sub(start.0);
            if condition() {
                return Ok(Microseconds(since_start));
            }
            if since_start >= timeout {
                return Err(Microseconds(since_start));
            }
        }
    }
}
