extern crate rodio;

use rodio::source::Source;
use std::io::BufReader;

fn main() {
    let device = rodio::default_output_device().unwrap();
    let (controller, queue_rx) = rodio::queue2::queue2(true);
    rodio::play_raw(&device, queue_rx);

    let file = std::fs::File::open("examples/music.flac").unwrap();
    let source = rodio::Decoder::new(BufReader::new(file)).unwrap();
    controller.append(source.convert_samples());

    let file = std::fs::File::open("examples/music.mp3").unwrap();
    let source = rodio::Decoder::new(BufReader::new(file)).unwrap();
    controller.append(source.convert_samples());

    let file = std::fs::File::open("examples/music.flac").unwrap();
    let source = rodio::Decoder::new(BufReader::new(file)).unwrap();
    controller.append(source.convert_samples());
    loop {
        std::thread::sleep_ms(3000);
        controller.next();
    }
}
