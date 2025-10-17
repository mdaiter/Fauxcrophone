use std::env;
use std::process;

use device_kit::LoopbackLevels;

fn print_status() {
    match device_kit::control::api::get_status() {
        Some(status) => {
            println!("Sample Rate : {} Hz", status.sample_rate);
            println!("Buffer Size : {} frames", status.buffer_frames);
            println!("Latency     : {:.2} ms", status.latency_ms);
            println!("CPU Usage   : {:.1}%", status.cpu_usage * 100.0);
            println!("Buffer Fill : {:.1}%", status.buffer_fill * 100.0);
            println!("Drift       : {:.1} ppm", status.drift_ppm);
            println!("Sources:");
            for source in status.sources {
                println!(
                    "  [{}] {} | gain={:.1} dB | mute={} | rms={:.2} | latency={} frames | fill={:.1}% | drift={:.1} ppm",
                    source.id,
                    source.name,
                    source.gain_db,
                    if source.muted { "yes" } else { "no" },
                    source.rms,
                    source.latency_frames,
                    source.buffer_fill * 100.0,
                    source.drift_ppm,
                );
            }

            let mut levels = LoopbackLevels {
                inputs: [0.0; 8],
                outputs: [0.0; 8],
                input_count: 0,
                output_count: 0,
            };
            if unsafe { device_kit::device_kit_get_levels(&mut levels as *mut LoopbackLevels) } {
                if levels.output_count > 0 {
                    println!("Output Levels:");
                    for (idx, level) in levels.outputs[..levels.output_count as usize]
                        .iter()
                        .enumerate()
                    {
                        println!("  Output {}: {:.2}", idx + 1, level);
                    }
                }
                if levels.input_count > 0 {
                    println!("Input Levels:");
                    for (idx, level) in levels.inputs[..levels.input_count as usize]
                        .iter()
                        .enumerate()
                    {
                        println!("  Input {}: {:.2}", idx + 1, level);
                    }
                }
            }
        }
        None => {
            eprintln!("loopbackctl: no active mixer detected");
            process::exit(1);
        }
    }
}

fn main() {
    let mut args = env::args().skip(1);
    if let Some(arg) = args.next() {
        match arg.as_str() {
            "--status" | "-s" => {
                print_status();
                return;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: loopbackctl [--status]\n\nWithout arguments the interactive console launches."
                );
                return;
            }
            other => {
                eprintln!("loopbackctl: unknown argument '{other}'");
                process::exit(1);
            }
        }
    }

    if let Err(err) = device_kit::control::ui::run() {
        eprintln!("loopbackctl: {err}");
        process::exit(1);
    }
}
