use std::process::Stdio;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let output = tokio::process::Command::new("/bin/sh")
        .args(["-c", "printf stdout-marker; printf stderr-marker >&2"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    assert!(output.status.success());
    assert_eq!(output.stdout, b"stdout-marker");
    assert_eq!(output.stderr, b"stderr-marker");
    println!("Tokio captured child stdout and stderr successfully");
    Ok(())
}