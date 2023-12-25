set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]
wokwi-run *args:
    export CARGO_TARGET_RISCV32IMC_ESP_ESPIDF_RUNNER="wokwi-server --chip esp32-c3 --id 383737049153055745"; cargo run {{args}}

wokwi-led *args:
    export CARGO_TARGET_RISCV32IMC_ESP_ESPIDF_RUNNER="wokwi-server --chip esp32-c3 --id 384845091910047745"; cargo run {{args}}