#[cfg(unix)]
use std::fs::Permissions;
use std::io::Write as _;
#[cfg(unix)]
use std::os::unix::prelude::PermissionsExt as _;
use std::process::Stdio;

use assert_matches::assert_matches;
use insta::assert_debug_snapshot;
use jj_lib::gpg_signing::GpgBackend;
use jj_lib::gpg_signing::GpgsmBackend;
use jj_lib::signing::SigStatus;
use jj_lib::signing::SignError;
use jj_lib::signing::SigningBackend as _;
use testutils::ensure_running_outside_ci;
use testutils::is_external_tool_installed;

static GPG_PRIVATE_KEY: &str = r#"-----BEGIN PGP PRIVATE KEY BLOCK-----

lFgEZWI3pBYJKwYBBAHaRw8BAQdAaPLTNADvDWapjAPlxaUnx3HXQNIlwSz4EZrW
3Z7hxSwAAP9liwHZWJCGI2xW+XNqMT36qpIvoRcd5YPaKYwvnlkG1w+UtDNTb21l
b25lIChqaiB0ZXN0IHNpZ25pbmcga2V5KSA8c29tZW9uZUBleGFtcGxlLmNvbT6I
kwQTFgoAOxYhBKWOXukGcVPI9eXp6WOHhcsW/qBhBQJlYjekAhsDBQsJCAcCAiIC
BhUKCQgLAgQWAgMBAh4HAheAAAoJEGOHhcsW/qBhyBgBAMph1HkBkKlrZmsun+3i
kTEaOsWmaW/D6NEdMFiw0S/jAP9G3jOYGiZbUN3dWWB2246Oi7SaMTX8Xb2BrLP2
axCbC5RYBGVjxv8WCSsGAQQB2kcPAQEHQE8Oa4ahtVG29gIRssPxjqF4utn8iHPz
m5z/8lX/nl3eAAD5AZ6H2pNhiy2gnGkbPLHw3ZyY4d0NXzCa7qc9EXqOj+sRrLQ9
U29tZW9uZSBFbHNlIChqaiB0ZXN0IHNpZ25pbmcga2V5KSA8c29tZW9uZS1lbHNl
QGV4YW1wbGUuY29tPoiTBBMWCgA7FiEER1BAaEpU3TKUiUvFTtVW6XKeAA8FAmVj
xv8CGwMFCwkIBwICIgIGFQoJCAsCBBYCAwECHgcCF4AACgkQTtVW6XKeAA/6TQEA
2DkPm3LmH8uG6qLirtf62kbG7T+qljIsarQKFw3CGakA/AveCtrL7wVSpINiu1Rz
lBqJFFP2PqzT0CRfh94HSIMM
=6JC8
-----END PGP PRIVATE KEY BLOCK-----
"#;

static GPGSM_FINGERPRINT: &str = "4C625C10FF7180164F19C6571D513E4E0BEA555C";

static GPGSM_PRIVATE_KEY: &str = r#"-----BEGIN PKCS12-----
MIIEjAIBAzCCBEIGCSqGSIb3DQEHAaCCBDMEggQvMIIEKzCCAuIGCSqGSIb3DQEHBqCCAtMwggLP
AgEAMIICyAYJKoZIhvcNAQcBMFcGCSqGSIb3DQEFDTBKMCkGCSqGSIb3DQEFDDAcBAhW4TA5N5aE
qAICCAAwDAYIKoZIhvcNAgkFADAdBglghkgBZQMEASoEEDyELdhdBjhSJgPcPmmdJQWAggJgR3zZ
ZHJQj2aoCDuPQrxBkklgnDmTF91bDStMX9J6B7ucFS2V7YEO1YcwfdphRRYRCkTO0L4/qLO5l/xg
R0CwchpOUbo9Xl6MHiRZW7nTEU2bO1oq45lTzIQfJtWK9R/Nujvx3KyTIm+2ZGBrVHZ301rmCepU
YtSBmtoo+9rlp+lkkvGh+E9+gWjvDhXUkaxkUjRvx/cdOeEKDM8SmfhX6nZ7lzbnI9xQ4d7g4Sn2
9Y3F0HHe5+qBwd97i4xL1fFQs9vKVe2Iqr46B6T++GuClR+66yjGHxeQ6qjMSAEk4kPP8/LPI5i0
xC15U38J8dOyXX1jNP9W44nu1CpiX7MEuEyeEel4mDq5HzbQp2AOeS6Zg4VSf8nz8uSES48DrPMw
lDFH/YCAWHEPgcTBqMKO0+EnVL4297WNKA8aJiD/tKZZEyS1SGqoXX5eHazZQHD9PReZBv0gTFSz
Aq/K+Gcrsh7I5/lhyuQ6gwbi2uluCdwJirRzc85RrO5GsBxDHdcngy9ez0duLsOf7UVgIku21PmD
d4ureqfT1rQZkE+hGXUc+NNF7ZTvCDHETCJwVgqqZttZ43ILT2yBAG7dV+X7AUNLn/LpZmZ6adIH
gyviuhleTMGoSnPJXCMkEnU00QoROo7yceSikjuaLV33HXEpcepOBRXW91r7DLQWLHT+mX2W8/oA
UX0UKQ2al0R9JrWsQOdGwNcbNHfRldAmRBW7ktOUyXlN71BE90TPjqA2Xu5Ta1yIs+XuU5BUAWzb
v9agzbfU4ZOa9FgSxExE6iQ+NkCuJ+05bHeVVqtbBgqurwswggFBBgkqhkiG9w0BBwGgggEyBIIB
LjCCASowggEmBgsqhkiG9w0BDAoBAqCB7zCB7DBXBgkqhkiG9w0BBQ0wSjApBgkqhkiG9w0BBQww
HAQIjo1upovnkrcCAggAMAwGCCqGSIb3DQIJBQAwHQYJYIZIAWUDBAEqBBBF0GsMP3O/uZs3/OHS
Fdl/BIGQmrK7oxltgZa0TihDJ7OVmCnbLawSB5E38Wjo7gSwPa2/1ofg8yU9ZBjdlYQRFevZcj1I
rU307BQIPmjqxIMSV8K/F1OfvWWrfRDXwvvn1CHNM4VuqfoJzwfYsD2jEedXAHN7a90sjtZeDqMs
ibOEeIIN2hOh6FBnaO2f4QVXTUoe4k0BJ2WTMtjoIJod0LKiMSUwIwYJKoZIhvcNAQkVMRYEFExi
XBD/cYAWTxnGVx1RPk4L6lVcMEEwMTANBglghkgBZQMEAgEFAAQgj7Jjd7XJ3icDiNTp080RDoUw
J+57G8w4qtRQPRTuOvcECGz+PguPT+pLAgIIAA==
-----END PKCS12-----
"#;

struct GpgEnvironment {
    homedir: tempfile::TempDir,
}

impl GpgEnvironment {
    fn new() -> Result<Self, std::process::Output> {
        let dir = tempfile::Builder::new()
            .prefix("gpg-test-")
            .tempdir()
            .unwrap();

        let path = dir.path();

        #[cfg(unix)]
        std::fs::set_permissions(path, Permissions::from_mode(0o700)).unwrap();

        let mut gpg = std::process::Command::new("gpg")
            .arg("--homedir")
            .arg(path)
            .arg("--import")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        gpg.stdin
            .as_mut()
            .unwrap()
            .write_all(GPG_PRIVATE_KEY.as_bytes())
            .unwrap();

        gpg.stdin.as_mut().unwrap().flush().unwrap();

        let res = gpg.wait_with_output().unwrap();

        if !res.status.success() {
            eprintln!("Failed to add private key to gpg-agent. Make sure it is running!");
            eprintln!("{}", String::from_utf8_lossy(&res.stderr));
            return Err(res);
        }

        Ok(Self { homedir: dir })
    }
}

struct GpgsmEnvironment {
    homedir: tempfile::TempDir,
}

impl GpgsmEnvironment {
    fn new() -> Result<Self, std::process::Output> {
        let dir = tempfile::Builder::new()
            .prefix("gpgsm-test-")
            .tempdir()
            .unwrap();

        let path = dir.path();

        #[cfg(unix)]
        std::fs::set_permissions(path, Permissions::from_mode(0o700)).unwrap();

        std::fs::write(
            path.join("trustlist.txt"),
            format!("{GPGSM_FINGERPRINT} S\n"),
        )
        .unwrap();

        let mut gpgsm = std::process::Command::new("gpgsm")
            .arg("--homedir")
            .arg(path)
            .arg("--batch")
            .arg("--pinentry-mode")
            .arg("loopback")
            .arg("--import")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();

        gpgsm
            .stdin
            .as_mut()
            .unwrap()
            .write_all(GPGSM_PRIVATE_KEY.as_bytes())
            .unwrap();

        gpgsm.stdin.as_mut().unwrap().flush().unwrap();

        let res = gpgsm.wait_with_output().unwrap();

        if !res.status.success() && res.status.code() != Some(2) {
            eprintln!("Failed to add certificate.");
            eprintln!("{}", String::from_utf8_lossy(&res.stderr));
            return Err(res);
        }

        Ok(Self { homedir: dir })
    }
}

macro_rules! gpg_guard {
    () => {
        if !is_external_tool_installed("gpg") {
            ensure_running_outside_ci("`gpg` must be in the PATH");
            eprintln!("Skipping test because gpg is not installed on the system");
            return;
        }
    };
}

macro_rules! gpgsm_guard {
    () => {
        if !is_external_tool_installed("gpgsm") {
            ensure_running_outside_ci("`gpgsm` must be in the PATH");
            eprintln!("Skipping test because gpgsm is not installed on the system");
            return;
        }
    };
}

fn gpg_backend(env: &GpgEnvironment) -> GpgBackend {
    // don't really need faked time for current tests,
    // but probably will need it for end-to-end cli tests
    GpgBackend::new("gpg".into(), false, "someone@example.com".to_owned()).with_extra_args(&[
        "--homedir".into(),
        env.homedir.path().as_os_str().into(),
        "--faked-system-time=1701042000!".into(),
    ])
}

fn gpgsm_backend(env: &GpgsmEnvironment) -> GpgsmBackend {
    // don't really need faked time for current tests,
    // but probably will need it for end-to-end cli tests
    GpgsmBackend::new("gpgsm".into(), false, "someone@example.com".to_owned()).with_extra_args(&[
        "--homedir".into(),
        env.homedir.path().as_os_str().into(),
        "--faked-system-time=1742477110!".into(),
    ])
}

#[test]
#[cfg_attr(windows, ignore = "stuck randomly on Windows CI #3140")] // FIXME
fn gpg_signing_roundtrip() {
    gpg_guard!();

    let env = GpgEnvironment::new().unwrap();
    let backend = gpg_backend(&env);
    let data = b"hello world";
    let signature = backend.sign(data, None).unwrap();

    let check = backend.verify(data, &signature).unwrap();
    assert_eq!(check.status, SigStatus::Good);
    assert_eq!(check.key.unwrap(), "638785CB16FEA061");
    assert_eq!(
        check.display.unwrap(),
        "Someone (jj test signing key) <someone@example.com>"
    );

    let check = backend.verify(b"so so bad", &signature).unwrap();
    assert_eq!(check.status, SigStatus::Bad);
    assert_eq!(check.key.unwrap(), "638785CB16FEA061");
    assert_eq!(
        check.display.unwrap(),
        "Someone (jj test signing key) <someone@example.com>"
    );
}

#[test]
#[cfg_attr(windows, ignore = "stuck randomly on Windows CI #3140")] // FIXME
fn gpg_signing_roundtrip_explicit_key() {
    gpg_guard!();

    let env = GpgEnvironment::new().unwrap();
    let backend = gpg_backend(&env);
    let data = b"hello world";
    let signature = backend.sign(data, Some("Someone Else")).unwrap();

    assert_debug_snapshot!(backend.verify(data, &signature).unwrap(), @r#"
    Verification {
        status: Good,
        key: Some(
            "4ED556E9729E000F",
        ),
        display: Some(
            "Someone Else (jj test signing key) <someone-else@example.com>",
        ),
    }
    "#);
    assert_debug_snapshot!(backend.verify(b"so so bad", &signature).unwrap(), @r#"
    Verification {
        status: Bad,
        key: Some(
            "4ED556E9729E000F",
        ),
        display: Some(
            "Someone Else (jj test signing key) <someone-else@example.com>",
        ),
    }
    "#);
}

#[test]
#[cfg_attr(windows, ignore = "stuck randomly on Windows CI #3140")] // FIXME
fn gpg_unknown_key() {
    gpg_guard!();

    let env = GpgEnvironment::new().unwrap();
    let backend = gpg_backend(&env);
    let signature = br"-----BEGIN PGP SIGNATURE-----

    iHUEABYKAB0WIQQs238pU7eC/ROoPJ0HH+PjJN1zMwUCZWPa5AAKCRAHH+PjJN1z
    MyylAP9WQ3sZdbC4b1C+/nxs+Wl+rfwzeQWGbdcsBMyDABcpmgD/U+4KdO7eZj/I
    e+U6bvqw3pOBoI53Th35drQ0qPI+jAE=
    =kwsk
    -----END PGP SIGNATURE-----";
    assert_debug_snapshot!(backend.verify(b"hello world", signature).unwrap(), @r#"
    Verification {
        status: Unknown,
        key: Some(
            "071FE3E324DD7333",
        ),
        display: None,
    }
    "#);
    assert_debug_snapshot!(backend.verify(b"so bad", signature).unwrap(), @r#"
    Verification {
        status: Unknown,
        key: Some(
            "071FE3E324DD7333",
        ),
        display: None,
    }
    "#);
}

#[test]
#[cfg_attr(windows, ignore = "stuck randomly on Windows CI #3140")] // FIXME
fn gpg_invalid_signature() {
    gpg_guard!();

    let env = GpgEnvironment::new().unwrap();
    let backend = gpg_backend(&env);
    let signature = br"-----BEGIN PGP SIGNATURE-----

    super duper invalid
    -----END PGP SIGNATURE-----";

    // Small data: gpg command will exit late.
    assert_matches!(
        backend.verify(b"a", signature),
        Err(SignError::InvalidSignatureFormat)
    );

    // Large data: gpg command will exit early because the signature is invalid.
    assert_matches!(
        backend.verify(&b"a".repeat(100 * 1024), signature),
        Err(SignError::InvalidSignatureFormat)
    );
}

#[test]
#[cfg_attr(windows, ignore = "stuck randomly on Windows CI #3140")] // FIXME
fn gpgsm_signing_roundtrip() {
    gpgsm_guard!();

    let env = GpgsmEnvironment::new().unwrap();
    let backend = gpgsm_backend(&env);
    let data = b"hello world";
    let signature = backend.sign(data, None);
    let signature = signature.unwrap();

    let check = backend.verify(data, &signature).unwrap();
    assert_eq!(check.status, SigStatus::Good);
    assert_eq!(check.key.unwrap(), GPGSM_FINGERPRINT);
    assert_eq!(
        check.display.unwrap(),
        "/CN=JJ Cert/O=GPGSM Signing Test/EMail=someone@example.com"
    );

    let check = backend.verify(b"so so bad", &signature).unwrap();
    assert_eq!(check.status, SigStatus::Bad);
    assert_eq!(check.key.unwrap(), GPGSM_FINGERPRINT);
    assert_eq!(
        check.display.unwrap(),
        "/CN=JJ Cert/O=GPGSM Signing Test/EMail=someone@example.com"
    );
}

#[test]
#[cfg_attr(windows, ignore = "stuck randomly on Windows CI #3140")] // FIXME
fn gpgsm_signing_roundtrip_explicit_key() {
    gpgsm_guard!();

    let env = GpgsmEnvironment::new().unwrap();
    let backend = gpgsm_backend(&env);
    let data = b"hello world";
    let signature = backend.sign(data, Some("someone@example.com")).unwrap();

    assert_debug_snapshot!(backend.verify(data, &signature).unwrap(), @r#"
    Verification {
        status: Good,
        key: Some(
            "4C625C10FF7180164F19C6571D513E4E0BEA555C",
        ),
        display: Some(
            "/CN=JJ Cert/O=GPGSM Signing Test/EMail=someone@example.com",
        ),
    }
    "#);
    assert_debug_snapshot!(backend.verify(b"so so bad", &signature).unwrap(), @r#"
    Verification {
        status: Bad,
        key: Some(
            "4C625C10FF7180164F19C6571D513E4E0BEA555C",
        ),
        display: Some(
            "/CN=JJ Cert/O=GPGSM Signing Test/EMail=someone@example.com",
        ),
    }
    "#);
}

#[test]
#[cfg_attr(windows, ignore = "stuck randomly on Windows CI #3140")] // FIXME
fn gpgsm_unknown_key() {
    gpgsm_guard!();

    let env = GpgsmEnvironment::new().unwrap();
    let backend = gpgsm_backend(&env);
    let signature = br"-----BEGIN SIGNED MESSAGE-----
    MIAGCSqGSIb3DQEHAqCAMIACAQExDzANBglghkgBZQMEAgEFADCABgkqhkiG9w0B
    BwEAADGCAnYwggJyAgEBMDUwKTEaMBgGA1UEChMRWDUwOSBTaWduaW5nIFRlc3Qx
    CzAJBgNVBAMTAkpKAgh8bds9GXiZmzANBglghkgBZQMEAgEFAKCBkzAYBgkqhkiG
    9w0BCQMxCwYJKoZIhvcNAQcBMBwGCSqGSIb3DQEJBTEPFw0yNTAzMTgyMDAzNDBa
    MCgGCSqGSIb3DQEJDzEbMBkwCwYJYIZIAWUDBAECMAoGCCqGSIb3DQMHMC8GCSqG
    SIb3DQEJBDEiBCCpSJBPLw9Hm4+Bl2lLMBhLDS7Rwc0qHsD7hdKZoZKkRzANBgkq
    hkiG9w0BAQEFAASCAYANOvWCJuOKn018s731TWFHq5wS13xB7L83/2q8Mi9cQ3YT
    kq8CQlyJV0spIW7dwztjsllX8X2szE4N0l83ghf3ol6B6n9Vyb844oKgb6cwc9uX
    S8D1yiaj1Mfft3PDp+THH+ESezw1Djzj7E53Yx5j3kna/ylJhheg3raWit2MUxI0
    V42Svm4PLcpOf+ywzstlSSx9p6Y8woctdkMkpyivNCsfwlRARFGSTP3G9DXZNv03
    WZ51zlMT8lsYbT9EJUxzXuEpcJZJL0TYcbJ3n7uSopivHk843onIc71gbH/ByuMp
    qokJ7jYzEMrk0YowzsD7wrtwhF5OgpW5ane8vuyquLOrRNX9H/TooE4+8OCM6nvQ
    w7jgv1/hsdtDnZCkVaM0plhb2btE7Awgol5M8f9IDz1Z+b0t4ydc/iqHtE9yaqvZ
    +aT9XXKKcj9XBhi1S790B4r8YoDyeiyzBs0gwvMuWjWMS7wixTbgx+IkQUrkgTLY
    xiNbRmGtEonl9d8JS/IAAAAAAAA=
    -----END SIGNED MESSAGE-----
    ";
    assert_debug_snapshot!(backend.verify(b"hello world", signature).unwrap(), @r#"
    Verification {
        status: Unknown,
        key: None,
        display: None,
    }
    "#);
    assert_debug_snapshot!(backend.verify(b"so bad", signature).unwrap(), @r#"
    Verification {
        status: Unknown,
        key: None,
        display: None,
    }
    "#);
}

#[test]
#[cfg_attr(windows, ignore = "stuck randomly on Windows CI #3140")] // FIXME
fn gpgsm_invalid_signature() {
    gpgsm_guard!();

    let env = GpgsmEnvironment::new().unwrap();
    let backend = gpgsm_backend(&env);
    let signature = br"-----BEGIN SIGNED MESSAGE-----
    super duper invalid
    -----END SIGNED MESSAGE-----";

    // Small data: gpgsm command will exit late.
    assert_matches!(
        backend.verify(b"a", signature),
        Err(SignError::InvalidSignatureFormat)
    );

    // Large data: gpgsm command will exit early because the signature is invalid.
    assert_matches!(
        backend.verify(&b"a".repeat(100 * 1024), signature),
        Err(SignError::InvalidSignatureFormat)
    );
}
