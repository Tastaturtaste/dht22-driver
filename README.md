# DHT22
No-std, no-dependency, platform-agnostic driver for the dht22 sensor
## Features
- [critical-section](https://crates.io/crates/critical-section) is used by default to prevent interrupts from messing with the timing-based data transfer protocol of the sensor. While it is recommended to use if possible, the driver may still work reliably if interrupts are disabled another way. Otherwise the chance for additional spurious invalid reads due to timing mismatches may be acceptable, as spurious failurs should be handled anyway. 
- `std` provides an implementation of `std::error::Error` for the `DhtError`, allowing more ergonomic error handling. When [error_in_core](https://github.com/rust-lang/rust/issues/103765) is stabelized this feature may be deprecated and possibly removed.
## Example usage
A specific example for the `esp32c3mini1` board is available in the examples folder. This example can also be run using [wokwi](https://wokwi.com/) (see the accompanying [justfile](https://github.com/casey/just) for further details how to run it). 

An example usage without any platform specific implementation is shown below.
```rust no_run
use dht22_driver::{IOPin, MicroTimer, Microseconds, Dht22};
// Newtype over device specific GPIO pin type
struct Pin;
#[derive(Debug)]
struct SomeMCUSpecificError;
impl IOPin for Pin
{
    type DeviceError = SomeMCUSpecificError;
    fn set_low(&mut self) -> Result<(), Self::DeviceError> {
        todo!()
    }
    fn set_high(&mut self) -> Result<(), Self::DeviceError> {
        todo!()
    }
    fn is_low(&self) -> bool {
        todo!()
    }
    fn is_high(&self) -> bool {
        todo!()
    }
}
// Newtype over device specific clock/timer
struct DeviceTimer;
impl MicroTimer for DeviceTimer {
    fn now(&self) -> Microseconds {
        Microseconds(
            todo!()
        )
    }
}
fn main() -> Result<(), SomeMCUSpecificError> {
    let mut pin = Pin;
    pin.set_high()?;
    let timer = DeviceTimer;
    let mut sensor = Dht22::new(pin, timer);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        if let Ok(reading) = sensor.read() {
            println!("Humidity: {:?}, Temperature: {:?}", reading.humidity, reading.temperature);    
        }
    }
}
```