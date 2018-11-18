// Copyright (c) 2017 repomons developers
//
// Licensed under the Apache License, Version 2.0
// <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0> or the MIT
// license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. All files in the project carrying such notice may not be copied,
// modified, or distributed except according to those terms.

//! `repomons` callbacks
use error::{Error, Result};
use git2::{self, Config, Cred, CredentialType, Progress, RemoteCallbacks};
use std::cell::RefCell;
use std::convert::TryFrom;
use std::rc::Rc;
use std::time::Instant;
use term;

/// The clone state.
#[derive(PartialEq)]
pub enum CloneState {
    /// Receiving objects.
    Receiving,
    /// Resolving deltas.
    Resolving,
}

/// Clone output shared state.
#[derive(Getters, MutGetters, Setters)]
pub struct CallbackOutput {
    /// The start instant.
    #[get = "pub"]
    start: Instant,
    /// The sideband callback output.
    #[get_mut = "pub"]
    sideband: String,
    /// The progress callback output.
    #[set = "pub"]
    #[get_mut = "pub"]
    progress: String,
    /// The current state.
    #[get = "pub"]
    #[set = "pub"]
    state: CloneState,
}

impl Default for CallbackOutput {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            sideband: String::new(),
            progress: String::new(),
            state: CloneState::Receiving,
        }
    }
}
/// Check credentials for connecting to remote.
pub fn check_creds(
    url: &str,
    username: Option<&str>,
    cred_type: CredentialType,
) -> ::std::result::Result<Cred, git2::Error> {
    if cred_type.contains(git2::CredentialType::SSH_KEY) {
        match Cred::ssh_key_from_agent(username.unwrap_or("")) {
            Ok(cred) => return Ok(cred),
            Err(_e) => {}
        }
    }

    if cred_type.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
        if let Ok(config) = Config::open_default() {
            match Cred::credential_helper(&config, url, username) {
                Ok(cred) => return Ok(cred),
                Err(_e) => {}
            }

            // TODO: Prompt for username/password here?
        }
    }

    Err(git2::Error::from_str("Unable to authenticate"))
}

/// Side band remote callback.
pub fn sideband(output: &mut CallbackOutput, text: &[u8]) -> bool {
    *output.sideband_mut() = String::from_utf8_lossy(text).into_owned();
    true
}

/// Generate a percent string from a numerator and denominator.
fn to_percent(num_pre: usize, dem_pre: usize) -> Result<String> {
    if dem_pre > 0 {
        let num_inter: u32 = num_pre as u32;
        let dem_inter: u32 = dem_pre as u32;
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
    let bytes = f64::from(curr_bytes as u32);
    let rem = f64::from(rem_pre as u32) / 1024.;
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
    let bytes = f64::from(bytes_pre as u32);
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
pub fn progress(output: &mut CallbackOutput, progress: &Progress) -> bool {
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
pub fn get_default<'a>(output: CallbackOutput) -> Result<RemoteCallbacks<'a>> {
    let shared_state: Rc<RefCell<_>> = Rc::new(RefCell::new(output));

    // Setup the progress callback.
    let progress_state = Rc::clone(&shared_state);
    let mut t = term::stdout().ok_or("unable to create stdout term")?;

    let progress_fn = move |progress_info: Progress| -> bool {
        let res = progress(&mut progress_state.borrow_mut(), &progress_info);
        let curr_state = progress_state.borrow();

        if !curr_state.progress.is_empty() {
            t.delete_line().expect("");
            write!(t, "{}", &curr_state.progress).expect("");
            t.carriage_return().expect("");
        }

        t.flush().expect("");
        res
    };

    // Setup the sideband callback.
    let sideband_state = Rc::clone(&shared_state);
    let mut st = term::stdout().ok_or("unable to create stdout term")?;
    let sideband_fn = move |bytes: &[u8]| -> bool {
        let res = sideband(&mut sideband_state.borrow_mut(), bytes);
        let curr_state = sideband_state.borrow();
        st.delete_line().expect("");
        write!(st, "{}", &curr_state.sideband).expect("");
        st.carriage_return().expect("");
        st.flush().expect("");
        res
    };

    let mut rcb = RemoteCallbacks::new();
    rcb.transfer_progress(progress_fn);
    rcb.sideband_progress(sideband_fn);
    rcb.credentials(check_creds);
    Ok(rcb)
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
        assert_eq!(super::bytes_to_string(1_048_575).expect(""), "1023.99 KiB");
        assert_eq!(super::bytes_to_string(1_048_576).expect(""), "1.00 MiB");
        assert_eq!(
            super::bytes_to_string(1_073_741_823).expect(""),
            "1023.99 MiB"
        );
        assert_eq!(super::bytes_to_string(1_073_741_824).expect(""), "1.00 GiB");
        assert_eq!(
            super::bytes_to_string(1_099_511_627_775).expect(""),
            "1023.99 GiB"
        );
        assert_eq!(
            super::bytes_to_string(1_099_511_627_776).expect(""),
            "1.00 TiB"
        );
        assert_eq!(
            super::bytes_to_string(1_125_899_906_842_623).expect(""),
            "1023.99 TiB"
        );
        assert_eq!(
            super::bytes_to_string(1_125_899_906_842_624).expect(""),
            "1.00 PiB"
        );
        assert_eq!(
            super::bytes_to_string(1_152_921_504_606_846_975).expect(""),
            "1023.99 PiB"
        );
        assert_eq!(
            super::bytes_to_string(1_152_921_504_606_846_976).expect(""),
            "1.00 EiB"
        );
        assert_eq!(
            super::bytes_to_string(usize::max_value()).expect(""),
            "15.01 EiB"
        );
    }
}
