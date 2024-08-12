#![no_std]
#![no_main]
use core::result::Result::Ok;
use core::result::Result::Err;

use embedded_tls::{Aes128GcmSha256, NoVerify, TlsConfig, TlsConnection, TlsContext};
use esp_backtrace as _;
use esp_hal::peripheral::Peripheral;
use esp_hal::{
    clock::ClockControl,
    peripherals::Peripherals,
    prelude::*,
    rng::Rng,
    system::SystemControl,
    timer::{timg::TimerGroup, ErasedTimer, OneShotTimer, PeriodicTimer},
};

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embassy_net::{tcp::TcpSocket, Config, Ipv4Address, Stack, StackResources};

use esp_println::println;
use esp_wifi::{
    initialize,
    wifi::{
        ClientConfiguration,
        Configuration,
        WifiController,
        WifiDevice,
        WifiEvent,
        WifiStaDevice,
        WifiState,
    },
    EspWifiInitFor,
};

// extern crate alloc;
// use core::mem::MaybeUninit;

// #[global_allocator]
// static ALLOCATOR: esp_alloc::EspHeap = esp_alloc::EspHeap::empty();
//
//  fn init_heap() {
//      const HEAP_SIZE: usize = 8* 1024;
//      static mut HEAP: MaybeUninit<[u8; HEAP_SIZE]> = MaybeUninit::uninit();
//
//      unsafe {
//          ALLOCATOR.init(HEAP.as_mut_ptr() as *mut u8, HEAP_SIZE);
//      }
//  }

macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
const PRINTER_IP: Ipv4Address = Ipv4Address::new(192, 168, 10, 78);
const PRINTER_PORT: u16 = 8883;

#[main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();

    let mut peripherals = Peripherals::take();
    // let peripherals = Peripherals::take();

    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();

    // init_heap();

    let timer = PeriodicTimer::new(
        esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG0, &clocks, None)
            .timer0
            .into(),
    );

    // Wifi
    
    let init =  initialize(
        EspWifiInitFor::Wifi,
        timer,
        unsafe { esp_hal::rng::Rng::new(peripherals.RNG.clone_unchecked()) },
        // Rng::new(peripherals.RNG),
        peripherals.RADIO_CLK,
        &clocks,
    )
    .unwrap();
    
    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

    // Embassy

    {
        let timg1 = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG1, &clocks, None);
        esp_hal_embassy::init(
            &clocks,
            mk_static!(
                [OneShotTimer<ErasedTimer>; 1],
                [OneShotTimer::new(timg1.timer0.into())]
            ),
        );
    }
    
    // Wifi again
    let config = Config::dhcpv4(Default::default());

     let seed = 1234; // very random, very secure seed

    // Init network stack
    let stack = &*mk_static!(
        Stack<WifiDevice<'_, WifiStaDevice>>,
        Stack::new(
            wifi_interface,
            config,
            mk_static!(StackResources<3>, StackResources::<3>::new()),
            seed
        )
    );

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(&stack)).ok();

    let mut rx_buffer = [0; 1024];
    let mut tx_buffer = [0; 1024];

    loop {
        println!("Waiting for link");
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    loop {
        println!("Waiting to get IP address...");
        if let Some(config) = stack.config_v4() {
            println!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    let mut rng = esp_hal::rng::Trng::new(peripherals.RNG, peripherals.ADC1);

    loop {
        Timer::after(Duration::from_millis(1_000)).await;

        let mut socket = TcpSocket::new(&stack, &mut rx_buffer, &mut tx_buffer);

        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        let remote_endpoint = (PRINTER_IP, PRINTER_PORT);
        println!("connecting...");
        let r = socket.connect(remote_endpoint).await;
        if let Err(e) = r {
            println!("connect error: {:?}", e);
            continue;
        }
        println!("connected!");

        // TLS
        let mut read_record_buffer = [0u8; 1024];
        let mut write_record_buffer = [0u8; 1024];

        /* Commenting out this section makes this work */

        let config = TlsConfig::new(); //.with_server_name("example.com");
        let mut tls: TlsConnection<TcpSocket, Aes128GcmSha256> =
            TlsConnection::new(socket, &mut read_record_buffer, &mut write_record_buffer);

         tls.open::</*OsRng*/esp_hal::rng::Trng, NoVerify>(TlsContext::new(&config, &mut rng))
             .await
             .expect("error establishing TLS connection");

        /* End of section */

        //
        // let mut buf = [0; 1024];
        // loop {
        //     use embedded_io_async::Write;
        //     // let r = socket
        //     //     .write_all(b"GET / HTTP/1.0\r\nHost: www.mobile-j.de\r\n\r\n")
        //     //     .await;
        //     // if let Err(e) = r {
        //     //     println!("write error: {:?}", e);
        //     //     break;
        //     // }
        //
        //     let n = match tls.read(&mut buf).await {
        //     // let n = match socket.read(&mut buf).await {
        //          Ok(0) => {
        //              println!("read EOF");
        //              break;
        //          }
        //          Ok(n) => n,
        //          Err(e) => {
        //              println!("read error: {:?}", e);
        //              break;
        //          }
        //      };
        //     println!("{}", core::str::from_utf8(&buf[..n]).unwrap());
        // }
        Timer::after(Duration::from_millis(3000)).await;
    }

}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        match esp_wifi::wifi::get_wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.try_into().unwrap(),
                password: PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
        println!("About to connect...");

        match controller.connect().await {
            Ok(_) => println!("Wifi connected!"),
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}
