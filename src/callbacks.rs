// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomons` callbacks
use error::{Error, Result};
use git2::{self, Cred, CredentialType, Progress, RemoteCallbacks};
use repo::{CloneOutput, CloneState};
use std::convert::TryFrom;
use std::time::Instant;

/// Check credentials for connecting to remote.
pub fn check_creds(
    _url: &str,
    username: Option<&str>,
    cred_type: CredentialType,
) -> ::std::result::Result<Cred, git2::Error> {
    if cred_type.contains(git2::SSH_KEY) {
        Cred::ssh_key_from_agent(username.unwrap_or(""))
    } else {
        Err(git2::Error::from_str("Unable to authenticate"))
    }
}

/// Side band remote callback.
pub fn sideband(output: &mut CloneOutput, text: &[u8]) -> bool {
    *output.sideband_mut() = String::from_utf8_lossy(text).into_owned();
    true
}

/// Generate a percent string from a numerator and denominator.
fn to_percent(num_pre: usize, dem_pre: usize) -> Result<String> {
    if dem_pre > 0 {
        let num_inter: u32 = TryFrom::try_from(num_pre)?;
        let dem_inter: u32 = TryFrom::try_from(dem_pre)?;
        let numerator: f64 = num_inter.into();
        let denominator: f64 = dem_inter.into();
        let result = (numerator / denominator) * 100.;
        Ok(format!("{}%", result.trunc()))
    } else {
        Err("cannot divide by 0".into())
    }
}

/// Supported byte units (that can fit in a usize).
enum ByteUnits {
    /// 0 to 1024^1 - 1
    Byte,
    /// 1024 to 1024^2 - 1
    Kibibyte,
    /// 1024^2 to 1024^3 - 1
    Mebibyte,
    /// 1024^3 to 1024^4 - 1
    Gibibyte,
    /// 1024^4 to 1024^5 - 1
    Tebibyte,
    /// 1024^5 to 1024^6 - 1
    Pebibyte,
    /// >1024^6
    Exbibyte,
}

impl TryFrom<usize> for ByteUnits {
    type Error = Error;

    fn try_from(idx: usize) -> Result<Self> {
        match idx {
            0 => Ok(ByteUnits::Byte),
            1 => Ok(ByteUnits::Kibibyte),
            2 => Ok(ByteUnits::Mebibyte),
            3 => Ok(ByteUnits::Gibibyte),
            4 => Ok(ByteUnits::Tebibyte),
            5 => Ok(ByteUnits::Pebibyte),
            6 => Ok(ByteUnits::Exbibyte),
            _ => Err("unsupported byte units index".into()),
        }
    }
}

/// Convert bytes to the maximum unit representation.
fn bytes_to_max_units(bytes_pre: usize) -> Result<(ByteUnits, usize, usize)> {
    let mut curr_bytes = bytes_pre;
    let mut rem_pre = 0;
    let mut unit_idx = 0;
    while curr_bytes >= 1024 {
        curr_bytes /= 1024;
        rem_pre = curr_bytes % 1024;
        unit_idx += 1;
        // ~1024^6 bytes is the max that can fit in a 64-bit usize, so break here.
        if unit_idx == 6 {
            break;
        }
    }
    Ok((TryFrom::try_from(unit_idx)?, curr_bytes, rem_pre))
}

/// Convert the bytes value to a properly unit-ed string.
fn bytes_to_string(bytes_pre: usize) -> Result<String> {
    let (units, curr_bytes, rem_pre) = bytes_to_max_units(bytes_pre)?;
    let bytes = f64::from(u32::try_from(curr_bytes)?);
    let rem = f64::from(u32::try_from(rem_pre)?) / 1024.;
    let two_decimal_down = (100. * (bytes + rem)).floor() / 100.;

    Ok(match units {
        ByteUnits::Byte => format!("{} B", two_decimal_down),
        ByteUnits::Kibibyte => format!("{:.2} KiB", two_decimal_down),
        ByteUnits::Mebibyte => format!("{:.2} MiB", two_decimal_down),
        ByteUnits::Gibibyte => format!("{:.2} GiB", two_decimal_down),
        ByteUnits::Tebibyte => format!("{:.2} TiB", two_decimal_down),
        ByteUnits::Pebibyte => format!("{:.2} PiB", two_decimal_down),
        ByteUnits::Exbibyte => format!("{:.2} EiB", two_decimal_down),
    })
}

/// Convert the current bytes to a rate, given the start time.
fn bytes_to_rate(bytes_pre: usize, start: &Instant) -> Result<String> {
    let elapsed = start.elapsed();
    let seconds =
        f64::from(u32::try_from(elapsed.as_secs())?) + f64::from(elapsed.subsec_nanos()) * 1e-9;
    let bytes = f64::from(u32::try_from(bytes_pre)?);
    let mut bytes_per_second = bytes / seconds;
    let mut unit_idx = 0;
    while bytes_per_second >= 1024. {
        bytes_per_second /= 1024.;
        unit_idx += 1;
        // ~1024^6 bytes is the max that can fit in a 64-bit usize, so break here.
        if unit_idx == 6 {
            break;
        }
    }
    let two_decimal_down = (100. * bytes_per_second).floor() / 100.;

    Ok(match ByteUnits::try_from(unit_idx)? {
        ByteUnits::Byte => format!("{} B/s", two_decimal_down),
        ByteUnits::Kibibyte => format!("{:.2} KiB/s", two_decimal_down),
        ByteUnits::Mebibyte => format!("{:.2} MiB/s", two_decimal_down),
        ByteUnits::Gibibyte => format!("{:.2} GiB/s", two_decimal_down),
        ByteUnits::Tebibyte => format!("{:.2} TiB/s", two_decimal_down),
        ByteUnits::Pebibyte => format!("{:.2} PiB/s", two_decimal_down),
        ByteUnits::Exbibyte => format!("{:.2} EiB/s", two_decimal_down),
    })
}

/// Progress remote callback.
#[cfg_attr(feature = "cargo-clippy", allow(needless_pass_by_value))]
pub fn progress(output: &mut CloneOutput, progress: Progress) -> bool {
    let received_objects = progress.received_objects();
    let total_objects = progress.total_objects();

    if received_objects < total_objects {
        let received_bytes = progress.received_bytes();
        let received_objects_percent = to_percent(received_objects, total_objects).expect("");
        let received_bytes_str = bytes_to_string(received_bytes).expect("");
        let rate = bytes_to_rate(received_bytes, output.start()).expect("");

        *output.progress_mut() = format!(
            "Receiving Objects: {} ({}/{}), {} | {}",
            received_objects_percent, received_objects, total_objects, received_bytes_str, rate
        );
    } else if received_objects == total_objects {
        match output.state() {
            CloneState::Receiving => {
                let received_bytes = progress.received_bytes();
                let received_objects_percent =
                    to_percent(received_objects, total_objects).expect("");
                let received_bytes_str = bytes_to_string(received_bytes).expect("");
                let rate = bytes_to_rate(received_bytes, output.start()).expect("");

                output.set_state(CloneState::Resolving);
                output.set_progress(format!(
                    "Receiving Objects: {} ({}/{}), {} | {}, done.\n",
                    received_objects_percent,
                    received_objects,
                    total_objects,
                    received_bytes_str,
                    rate
                ));
            }
            CloneState::Resolving => {
                let indexed_deltas = progress.indexed_deltas();
                let total_deltas = progress.total_deltas();
                let deltas_percent = if total_deltas == 0 {
                    "0%".to_string()
                } else {
                    to_percent(indexed_deltas, total_deltas).expect("")
                };
                if indexed_deltas < total_deltas || indexed_deltas == 0 {
                    output.set_progress(format!(
                        "Resolving Deltas: {} ({}/{})",
                        deltas_percent, indexed_deltas, total_deltas
                    ));
                } else {
                    output.set_progress(format!(
                        "Resolving Deltas: {} ({}/{}), done.\n",
                        deltas_percent, indexed_deltas, total_deltas
                    ));
                }
            }
        }
    }

    true
}

/// Setup the default set of callbacks.
pub fn get_default<'a>() -> RemoteCallbacks<'a> {
    let mut rcb = RemoteCallbacks::new();
    // rcb.transfer_progress(progress);
    // rcb.sideband_progress(side_band);
    rcb.credentials(check_creds);
    rcb
}

#[cfg(test)]
mod test {
    #[test]
    fn to_percent() {
        assert_eq!(super::to_percent(25, 100).expect("invalid percent"), "25%");
        assert_eq!(super::to_percent(50, 100).expect("invalid percent"), "50%");
        assert_eq!(super::to_percent(75, 100).expect("invalid percent"), "75%");
        assert_eq!(super::to_percent(1, 100).expect("invalid percent"), "1%");
        assert_eq!(super::to_percent(1, 1000).expect("invalid percent"), "0%");
        assert_eq!(super::to_percent(10, 1000).expect("invalid percent"), "1%");
    }

    #[test]
    fn bytes_to_string() {
        assert_eq!(super::bytes_to_string(512).expect(""), "512 B");
        assert_eq!(super::bytes_to_string(1023).expect(""), "1023 B");
        assert_eq!(super::bytes_to_string(1024).expect(""), "1.00 KiB");
        assert_eq!(super::bytes_to_string(1048575).expect(""), "1023.99 KiB");
        assert_eq!(super::bytes_to_string(1048576).expect(""), "1.00 MiB");
        assert_eq!(super::bytes_to_string(1073741823).expect(""), "1023.99 MiB");
        assert_eq!(super::bytes_to_string(1073741824).expect(""), "1.00 GiB");
        assert_eq!(
            super::bytes_to_string(1099511627775).expect(""),
            "1023.99 GiB"
        );
        assert_eq!(super::bytes_to_string(1099511627776).expect(""), "1.00 TiB");
        assert_eq!(
            super::bytes_to_string(1125899906842623).expect(""),
            "1023.99 TiB"
        );
        assert_eq!(
            super::bytes_to_string(1125899906842624).expect(""),
            "1.00 PiB"
        );
        assert_eq!(
            super::bytes_to_string(1152921504606846975).expect(""),
            "1023.99 PiB"
        );
        assert_eq!(
            super::bytes_to_string(1152921504606846976).expect(""),
            "1.00 EiB"
        );
        assert_eq!(
            super::bytes_to_string(usize::max_value()).expect(""),
            "15.01 EiB"
        );
    }
}
