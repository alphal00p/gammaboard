fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=GAMMABOARD_GIT_REVISION");

    if let Ok(revision) = std::env::var("GAMMABOARD_GIT_REVISION") {
        println!("cargo:rustc-env=GAMMABOARD_GIT_REVISION={revision}");
        return;
    }

    let revision = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(revision) = revision {
        println!("cargo:rustc-env=GAMMABOARD_GIT_REVISION={revision}");
    }
}
