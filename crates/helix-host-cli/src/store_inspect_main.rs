use helix_host_cli::store_inspect::{inspect, InspectRequest};

fn main() {
    let request = match InspectRequest::parse(std::env::args().skip(1)) { Ok(value) => value, Err(error) => { eprintln!("{error}"); std::process::exit(2); } };
    match inspect(&request) { Ok(result) => println!("{}", result.json), Err(error) => { eprintln!("{error}"); std::process::exit(1); } }
}
