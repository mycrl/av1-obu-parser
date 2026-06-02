use std::path::Path;

use av1_obu_parser::{
    IvfReader,
    buffer::Buffer,
    obu::{Obu, ObuParser},
};
use clap::Parser;
use tokio::fs;

#[derive(Parser)]
#[command(
    about = env!("CARGO_PKG_DESCRIPTION"),
    version = env!("CARGO_PKG_VERSION"),
    author = env!("CARGO_PKG_AUTHORS"),
)]
struct Configure {
    #[arg(long, default_value = "./assets/sample.ivf")]
    input: String,

    #[arg(long, default_value_t = 10)]
    obus: usize,

    #[arg(long)]
    show_delimiter: bool,
}

#[tokio::main]
async fn main() {
    let config = Configure::parse();
    let path = Path::new(&config.input);
    let file_data = fs::read(path)
        .await
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let ivf = IvfReader::new(&file_data).unwrap_or_else(|err| {
        panic!(
            "failed to parse IVF container from {}: {err}",
            path.display()
        )
    });

    let header = ivf.header();
    println!(
        "Reading {}: codec={}, resolution={}x{}",
        path.display(),
        header.codec_string(),
        header.width,
        header.height
    );

    let mut parser = ObuParser::default();
    let mut obu_count = 0usize;

    'frames: for ivf_frame in ivf.frames() {
        let ivf_frame = ivf_frame.expect("failed to read IVF frame");

        println!(
            "Frame {} pts={} size={}",
            ivf_frame.index,
            ivf_frame.pts,
            ivf_frame.data.len()
        );

        let mut buffer = Buffer::from_slice(ivf_frame.data);
        while buffer.bytes_remaining() > 0 {
            match parser.parse(&mut buffer) {
                Ok(Obu::TemporalDelimiter) => {
                    obu_count += 1;
                    if config.show_delimiter {
                        println!("OBU #{obu_count}: TemporalDelimiter");
                    }
                }
                Ok(Obu::Drop) => {}
                Ok(obu) => {
                    obu_count += 1;
                    println!("OBU #{obu_count}: {obu:#?}");
                }
                Err(err) => {
                    eprintln!("frame {}: parse error: {err:?}", ivf_frame.index);
                    break;
                }
            }

            if obu_count >= config.obus {
                break 'frames;
            }
        }
    }
}
