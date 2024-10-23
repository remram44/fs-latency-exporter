use prometheus::{Counter, Histogram, HistogramOpts, Opts};
use rand::Rng;
use std::env::args_os;
use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::exit;
use std::time::{Duration, Instant};
use tracing::{error, info};

fn parse_option<R: std::str::FromStr>(opt: Option<OsString>, flag: &'static str) -> R {
    let opt = match opt {
        Some(o) => o,
        None => {
            eprintln!("Missing value for {}", flag);
            exit(2);
        }
    };
    if let Some(opt) = opt.to_str() {
        if let Ok(opt) = opt.parse() {
            return opt;
        }
    }
    eprintln!("Invalid value for {}", flag);
    exit(2);
}

fn main() {
    // Initialize logging
    pretty_env_logger::init();

    // Parse command line
    let mut filename: Option<PathBuf> = None;
    let mut interval = 1.0;
    let mut metrics_addr: std::net::SocketAddr = ([0, 0, 0, 0], 8080).into();

    let mut args = args_os();
    args.next();
    let usage = "\
Usage: fs-latency-exporter [options] FILENAME
Options:
    --interval SECONDS
        Perform a measurement once every SECONDS minimum
    --metrics PORT
        Expose the statistics on HTTP PORT (default: 8080)";
    while let Some(arg) = args.next() {
        if &arg == "--help" {
            println!("{}", usage);
            exit(0);
        } else if &arg == "--interval" {
            interval = parse_option(args.next(), "--interval");
        } else if &arg == "--metrics" {
            metrics_addr = parse_option(args.next(), "--metrics");
        } else {
            if filename.is_none() {
                filename = Some(arg.into());
            } else {
                eprintln!("Too many arguments");
                eprintln!("{}", usage);
                exit(2);
            }
        }
    }

    let filename = match filename {
        Some(n) => n,
        None => {
            eprintln!("Missing filename");
            eprintln!("{}", usage);
            exit(2);
        }
    };

    // Set up Prometheus
    let errors_opts = Opts::new("errors_total", "Number of read errors");
    let errors = Counter::with_opts(errors_opts).unwrap();
    prometheus::default_registry()
        .register(Box::new(errors.clone()))
        .unwrap();
    let latency_opts = HistogramOpts::new("read_time_seconds", "Time taken to read (latency)");
    let latency_opts = latency_opts.buckets(vec![
        0.0001,
        0.00025, 0.0005, 0.001,
        0.0025, 0.005, 0.01,
        0.025, 0.05, 0.1,
        0.25, 0.5, 1.0,
    ]);
    let latency = Histogram::with_opts(latency_opts).unwrap();
    prometheus::default_registry()
        .register(Box::new(latency.clone()))
        .unwrap();

    // Start metrics server thread
    {
        use prometheus::Encoder;
        use tokio::runtime::Builder;
        use warp::Filter;

        std::thread::spawn(move || {
            info!("Starting Prometheus HTTP server on {}", metrics_addr);

            let rt = Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let routes = warp::path("metrics").map(move || {
                    let mut buffer = Vec::new();
                    let encoder = prometheus::TextEncoder::new();
                    let metric_families = prometheus::gather();
                    encoder.encode(&metric_families, &mut buffer).unwrap();
                    buffer
                });
                warp::serve(routes).run(metrics_addr).await;
            });
        });
    }

    // Open file (for direct I/O on UNIX)
    let mut opener = OpenOptions::new();
    opener.read(true);
    #[cfg(target_family = "unix")]
    {
        const O_DIRECT: i32 = 0x4000;
        opener.custom_flags(O_DIRECT);
    }
    let mut file = match opener.open(&filename) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Can't open {:?}: {}", filename, e);
            exit(1);
        }
    };
    let file_size = match file.metadata() {
        Ok(m) => m.len(),
        Err(e) => {
            eprintln!("Can't read file length: {}", e);
            exit(1);
        }
    };
    if file_size < 4096 {
        eprintln!("File is too small: {} bytes", file_size);
        exit(1);
    }
    info!("Opened {:?}, size {}", filename, file_size);

    let mut rng = rand::thread_rng();

    // Make an aligned buffer
    let mut buffer = vec![0; 8192];
    let buffer = {
        let ptr: *const u8 = (&mut buffer[0]) as &mut u8 as *const u8;
        let ptr: usize = ptr as usize;
        let padding = 4096 - ptr % 4096;
        &mut buffer[padding..padding + 4096]
    };
    assert_eq!(buffer.len(), 4096);

    loop {
        // Pick random offset in the file
        let offset = rng.gen_range(0..file_size / 4096) * 4096;

        let start = Instant::now();

        // Read
        match file.seek(SeekFrom::Start(offset)) {
            Err(e) => {
                error!("Error seeking to {}: {}", offset, e);
                errors.inc();
            }
            Ok(_) => match file.read_exact(buffer) {
                Ok(()) => {
                    latency.observe(start.elapsed().as_secs_f64());
                }
                Err(e) => {
                    error!("Error reading at offset {}: {}", offset, e);
                    errors.inc();
                }
            },
        }

        // Wait before next measurement
        std::thread::sleep(Duration::from_secs_f32(interval));
    }
}
