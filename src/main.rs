use std::time::Duration;

use embedded_svc::wifi::{AuthMethod, ClientConfiguration, Configuration};
use esp_idf_hal::gpio::PinDriver;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::EspWifi;
use esp_idf_sys::EspError;

mod dht22;
mod util;

fn main() -> Result<(), EspError> {
    esp_idf_sys::link_patches();

    let mut peripherals = Peripherals::take().unwrap();
    let sysloop = EspSystemEventLoop::take()?;
    let nvs_default_partition = EspDefaultNvsPartition::take()?;

    // let mut wifi = EspWifi::new(
    //     peripherals.modem,
    //     sysloop.clone(),
    //     Some(nvs_default_partition.clone()),
    // )?;

    // wifi.set_configuration(&Configuration::Client(ClientConfiguration {
    //     ssid: "Wokwi-GUEST".into(),
    //     password: "".into(),
    //     auth_method: AuthMethod::None,
    //     ..Default::default()
    // }))?;
    // wifi.start()?;
    // wifi.connect()?;
    // Safety: This is the only instanciation of this GPIO pin
    // let mut sensor = dht22::Dht22::new(peripherals.pins.gpio2, peripherals.timer00);
    // let number_tries = 20;
    // for _ in 0..number_tries {
    //     match sensor.read() {
    //         Ok(result) => {
    //             println!(
    //                 "Humidity: {}, Temperature: {}",
    //                 result.humidity, result.temperature
    //             );
    //             break;
    //         }
    //         Err(err) => println!("{err:#?}"),
    //     }
    // }
    let waiter = dht22::Waiter::new();
    let section = esp_idf_hal::interrupt::IsrCriticalSection::new();
    let pin = peripherals.pins.gpio3;
    let mut pin = PinDriver::input_output_od(pin).unwrap();
    pin.set_high().unwrap();
    for _ in 0..20 {
        let measured_time;
        let time0;
        let time1;
        {
            let _section_guard = section.enter();
            time0 = waiter.timer.now();
            pin.set_low().unwrap();
            measured_time = waiter
                .wait_for(|| false, Duration::from_micros(5000))
                .unwrap_err();
            time1 = waiter.timer.now();
            pin.set_high().unwrap();
        }
        println!(
            "Measured Time: {:#?}, Bracketed Time: {:#?}",
            measured_time,
            time1 - time0
        );
    }
    Ok(())
}
