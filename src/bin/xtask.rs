use std::{env, path::Path, process::Command};

fn main() {
    if let Err(error) = verify() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn verify() -> Result<(), String> {
    if env::args().skip(1).collect::<Vec<_>>() != ["verify"] {
        return Err("usage: cargo xtask verify".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for arguments in [
        vec!["fmt", "--", "--check"],
        vec![
            "clippy",
            "--locked",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        vec!["test", "--locked"],
    ] {
        let status = Command::new("cargo")
            .args(&arguments)
            .current_dir(root)
            .status()
            .map_err(|error| format!("could not run cargo: {error}"))?;
        if !status.success() {
            return Err(format!(
                "verification failed: cargo {} ({status})",
                arguments[0]
            ));
        }
    }
    println!("Verification passed.");
    Ok(())
}
