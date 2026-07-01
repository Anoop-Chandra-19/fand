//! fanctl — unprivileged CLI client (user must be in the `fand` group).
//!
//! Commands (plan §6):
//!   fanctl status                     table of temps, RPMs, PWMs
//!   fanctl watch                      live view via subscribe_status
//!   fanctl curve show/set <name>      inspect/edit curves
//!   fanctl override pwm2 140 --ttl 60
//!   fanctl config edit | reload

fn main() {
    // TODO phase 4: clap subcommands + socket client over fand-proto.
    eprintln!("fanctl: not implemented yet (phase 4)");
    std::process::exit(1);
}
