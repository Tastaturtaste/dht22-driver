#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/README.md"))]

#[cfg(feature = "critical-section")]
use critical_section::with;
#[cfg(not(feature = "critical-section"))]
fn with<R>(f: impl FnOnce(()) -> R) -> R {
    f(())
}

#[derive(Debug)]
pub enum DhtError<DeviceError> {
    /// Initial handshake with the sensor was unsuccessful. Make sure all physical connections are working, individual reads of the sensor are seperated by at least 2 seconds and the pin state is high while idle
    Handshake,
    /// Timeout while waiting for the sensor to respond
    Timeout(Microseconds),
    /// The checksum of the read data does not match with the provided checksum
    Checksum { correct: u8, actual: u8 },
    /// While setting the pin state the DeviceError occured
    DeviceError(DeviceError),
}

impl<DeviceError> core::fmt::Display for DhtError<DeviceError>
where
    DeviceError: core::fmt::Display,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            DhtError::Handshake => write!(f, "Inital handshake failed!"),
            DhtError::Timeout(us) => write!(
                f,
                "Timeout while waiting for level change after {} microseconds",
                us.0
            ),
            DhtError::Checksum { correct, actual } => write!(
                f,
                "Checksum validation failed. Correct: {correct}, Actual: {actual}"
            ),
            DhtError::DeviceError(device_error) => write!(f, "DeviceError: {device_error}"),
        }
    }
}

#[cfg(feature = "std")]
impl<DeviceError> std::error::Error for DhtError<DeviceError> where DeviceError: std::error::Error {}

impl<DeviceError> From<DeviceError> for DhtError<DeviceError> {
    fn from(value: DeviceError) -> Self {
        Self::DeviceError(value)
    }
}

/// Represents a GPIO pin capable of reading and setting the voltage level
pub trait IOPin {
    type DeviceError;
    fn set_low(&mut self) -> Result<(), Self::DeviceError>;
    fn set_high(&mut self) -> Result<(), Self::DeviceError>;
    fn is_low(&self) -> bool;
    fn is_high(&self) -> bool;
}

/// Represents a number of microseconds.
/// Simple Newtype to attach meaning to the contained primitive.
/// The std::duration::Duration which could also be used here is a much larger type in order to accomodate much
/// bigger time spans, which may impact performance, code size and stack usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Microseconds(pub u32);

/// Represents a timer with microsecond resolution
pub trait MicroTimer {
    /// Return an instance in time as a number of microseconds since some fixed point (normally boot or start of the timer).
    /// The implementation is allowed to wrap during a read from the sensor.
    fn now(&self) -> Microseconds;
}

/// Represents a DHT22 sensor connected to a pin.
pub struct Dht22<Pin, Timer>
where
    Pin: IOPin,
    Timer: MicroTimer,
{
    pin: Pin,
    timer: Timer,
}

/// A valid reading from the DHT22 sensor
pub struct SensorReading {
    pub humidity: f32,
    pub temperature: f32,
}

impl<Pin, Timer> Dht22<Pin, Timer>
where
    Pin: IOPin,
    <Pin as IOPin>::DeviceError: core::fmt::Debug,
    Timer: MicroTimer,
{
    /// Construct a new representation of the DHT22 sensor.
    /// Construction is cheap as long as the pin and clock are cheap to move.
    pub fn new(pin: Pin, clock: Timer) -> Self {
        Self { pin, timer: clock }
    }
    /// Attempt one read from the DHT22 sensor.
    /// Between subsequent reads from the same sensor at least 2 seconds should pass to avoid erratic readings.
    /// Reading to early after startup may also result in failure to read.
    pub fn read(&mut self) -> Result<SensorReading, DhtError<Pin::DeviceError>> {
        const RESPONSE_BITS: usize = 40;
        // Each bit is indicated by the two edges of the HIGH level (up, down).
        // In addition the initial down edge from the get-ready HIGH state is recorded.
        let mut cycles: [u32; 2 * RESPONSE_BITS + 1] = [0; 2 * RESPONSE_BITS + 1];
        let waiter = Waiter { timer: &self.timer };
        // Disable interrupts while interacting with the sensor so they don't mess up the timings
        with(|_guard| {
            // Initial handshake
            self.pin.set_low()?;
            let _ = waiter.wait_for(|| false, 1200);
            self.pin.set_high()?;

            // Wait for DHT22 to acknowledge the handshake with low
            if waiter.wait_for(|| self.pin.is_low(), 100).is_err() {
                return Err(DhtError::Handshake);
            }

            // Wait for low to end
            if waiter.wait_for(|| self.pin.is_high(), 100).is_err() {
                return Err(DhtError::Handshake);
            }
            // Data transfer started. Each bit starts with 50us low and than ~27us high for a 0 or ~70us high for a 1.
            // The pin should stay high for about 80us before transmission starts, we don't actually care about the precise timing of this duration.
            // To be precise just record time of edges and process later.
            let mut is_high = true;
            for duration in &mut cycles {
                *duration = waiter
                    .wait_for(|| is_high != self.pin.is_high(), 100)
                    .map(|microsecond| microsecond.0)
                    .map_err(DhtError::Timeout)?;

                is_high = !is_high;
            }
            // Data transfer ended
            Ok(())
        })
        .map_err(|err| {
            // Reset pin to high, which is the idle state of the dht22
            // Ignore any error that might occur here so the user sees the first error that occured
            let _ = self.pin.set_high();
            err
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
        let correct = bytes[4];
        let actual = bytes[0]
            .wrapping_add(bytes[1])
            .wrapping_add(bytes[2])
            .wrapping_add(bytes[3]);
        if actual != correct {
            return Err(DhtError::Checksum { actual, correct });
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

struct Waiter<'timer, Timer>
where
    Timer: MicroTimer,
{
    timer: &'timer Timer,
}
impl<'timer, Timer> Waiter<'timer, Timer>
where
    Timer: MicroTimer,
{
    #[inline(always)]
    fn wait_for(
        &self,
        condition: impl Fn() -> bool,
        timeout: u32,
    ) -> Result<Microseconds, Microseconds> {
        let start = self.timer.now();
        loop {
            // Using wrapping arithmetic on unsigned integers, overflow of the timer can be
            // exploited to count over the whole representable range of the integer type regardless of initial value.
            // For example for a u8:
            // 10 - 230 = 36
            let since_start = self.timer.now().0.wrapping_sub(start.0);
            if condition() {
                return Ok(Microseconds(since_start));
            }
            if since_start >= timeout {
                return Err(Microseconds(since_start));
            }
        }
    }
}
