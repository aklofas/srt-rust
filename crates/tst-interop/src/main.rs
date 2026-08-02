use std::env;

fn usage() -> String {
    "usage: tst-interop <subcommand> [options...]

Subcommands:
  gen       Generate synthetic test fixtures
  send      Send test data to endpoint
  recv      Receive test data from endpoint
  verify    Verify interop test results
  proxy     Proxy between endpoints
  report    Generate interop report

Options:
  -h, --help   Show this help message"
        .to_string()
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        println!("{}", usage());
        std::process::exit(0);
    }

    let subcommand = &args[1];

    match subcommand.as_str() {
        "-h" | "--help" => {
            println!("{}", usage());
            std::process::exit(0);
        }
        "gen" => {
            eprintln!("gen: not implemented");
            std::process::exit(2);
        }
        "send" => {
            eprintln!("send: not implemented");
            std::process::exit(2);
        }
        "recv" => {
            eprintln!("recv: not implemented");
            std::process::exit(2);
        }
        "verify" => {
            eprintln!("verify: not implemented");
            std::process::exit(2);
        }
        "proxy" => {
            eprintln!("proxy: not implemented");
            std::process::exit(2);
        }
        "report" => {
            eprintln!("report: not implemented");
            std::process::exit(2);
        }
        _ => {
            eprintln!("Unknown subcommand: {}", subcommand);
            println!("{}", usage());
            std::process::exit(2);
        }
    }
}
