use dht22::IOPin;
use esp_idf_hal::{
    gpio::{InputOutput, PinDriver},
    peripheral::Peripheral,
    peripherals::Peripherals,
    sys::EspError,
    timer::{config::Config, Timer, TimerDriver},
};

struct Pin<'pin, P: esp_idf_hal::gpio::Pin>(esp_idf_hal::gpio::PinDriver<'pin, P, InputOutput>);

impl<'a, P> IOPin for Pin<'a, P>
where
    P: esp_idf_hal::gpio::IOPin,
{
    type DeviceError = EspError;
    fn set_low(&mut self) -> Result<(), EspError> {
        self.0.set_low()?;
        Ok(())
    }

    fn set_high(&mut self) -> Result<(), EspError> {
        self.0.set_high()?;
        Ok(())
    }

    fn is_low(&self) -> bool {
        self.0.is_low()
    }

    fn is_high(&self) -> bool {
        self.0.is_high()
    }
}

struct MicroTimer<'timer>(TimerDriver<'timer>);

impl<'timer> MicroTimer<'timer> {
    fn new(timer: impl Peripheral<P = impl Timer> + 'timer) -> Result<Self, EspError> {
        Ok(Self(TimerDriver::new(timer, &Config::new())?))
    }
}

// The timer uses the APB_CLK which typically ticks with 80 MHz https://docs.espressif.com/projects/esp-idf/en/latest/esp32c3/api-reference/system/system_time.html
// and per default uses a divider of 80. Therefor the timer tick frequency is 1MHz.
impl<'timer> dht22::MicroTimer for MicroTimer<'timer> {
    fn now(&self) -> dht22::Microseconds {
        dht22::Microseconds(
            TryFrom::try_from(self.0.counter().expect("Could not read timer counter!"))
                .expect("Overflow while converting timer ticks from u64 to u32 "),
        )
    }
}

fn main() -> Result<(), EspError> {
    esp_idf_sys::link_patches();

    let peripherals = Peripherals::take().unwrap();
    let mut pin = Pin(PinDriver::input_output_od(peripherals.pins.gpio2)?);
    pin.set_high()?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    let clock = MicroTimer::new(peripherals.timer00)?;
    let mut sensor = dht22::Dht22::new(pin, clock);
    loop {
        match sensor.read() {
            Ok(result) => {
                println!(
                    "Humidity: {}, Temperature: {}",
                    result.humidity, result.temperature
                );
            }
            Err(err) => println!("{err}"),
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    Ok(())
}
