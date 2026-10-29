//! check-overlap: the close-copying check of a draft against the library notes (see the library's overlap.rs).
//! Exit status 0: no run found; 1: runs found; 2: the check could not run.

use authenticity_native::overlap;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let a = match overlap::parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    match overlap::check(&a) {
        Ok(out) => {
            overlap::print(&a, &out);
            std::process::exit(i32::from(!out.runs.is_empty()));
        }
        Err(e) => {
            eprintln!("check-overlap: {e}");
            std::process::exit(2);
        }
    }
}
