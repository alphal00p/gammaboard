fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/packed-refs");
    println!("cargo:rerun-if-env-changed=GAMMABOARD_GIT_REVISION");

    if let Ok(head) = std::fs::read_to_string(".git/HEAD")
        && let Some(reference) = head.trim().strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed=.git/{reference}");
    }

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
    if let Some(mut revision) = revision {
        let dirty = std::process::Command::new("git")
            .args(["status", "--porcelain", "--untracked-files=no"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .is_some_and(|output| !output.stdout.is_empty());
        if dirty {
            revision.push_str("-dirty");
        }
        println!("cargo:rustc-env=GAMMABOARD_GIT_REVISION={revision}");
    }
}
