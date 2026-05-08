use std::env;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    std::process::exit(cxs::shim_rg_main(&args));
}
