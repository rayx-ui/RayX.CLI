use crate::app::BuildProfile;
use anyhow::{Result, bail};

pub fn take_flag_value(args: &mut Vec<String>, flag: &str) -> Result<Option<String>> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    let _ = args.remove(index);
    if index < args.len() {
        Ok(Some(args.remove(index)))
    } else {
        bail!("{flag} expects a value")
    }
}

pub fn take_bool_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return false;
    };
    let _ = args.remove(index);
    true
}

pub fn take_bool_flag_value(args: &mut Vec<String>, flag: &str) -> Result<Option<bool>> {
    let Some(value) = take_flag_value(args, flag)? else {
        return Ok(None);
    };
    Ok(Some(parse_bool_flag_value(flag, &value)?))
}

pub fn take_profile(args: &mut Vec<String>) -> Result<BuildProfile> {
    if take_bool_flag(args, "--release") {
        bail!("release is the default build mode; remove --release or pass --debug/--development");
    }
    let debug = take_bool_flag(args, "--debug");
    let development = take_bool_flag(args, "--development");
    if debug && development {
        bail!("use only one development build flag: --debug or --development");
    }
    if debug || development {
        Ok(BuildProfile::Debug)
    } else {
        Ok(BuildProfile::Release)
    }
}

pub fn ensure_empty(args: &[String]) -> Result<()> {
    if args.is_empty() {
        Ok(())
    } else {
        bail!("unexpected argument(s): {}", args.join(" "))
    }
}

fn parse_bool_flag_value(flag: &str, value: &str) -> Result<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Ok(true),
        "false" | "0" | "no" | "n" | "off" => Ok(false),
        _ => bail!("{flag} expects a boolean value such as true or false"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_defaults_to_release() -> Result<()> {
        let mut args = Vec::new();

        assert_eq!(take_profile(&mut args)?, BuildProfile::Release);
        assert!(args.is_empty());
        Ok(())
    }

    #[test]
    fn debug_flag_selects_debug_profile() -> Result<()> {
        let mut args = vec!["--debug".to_string()];

        assert_eq!(take_profile(&mut args)?, BuildProfile::Debug);
        assert!(args.is_empty());
        Ok(())
    }

    #[test]
    fn development_flag_selects_debug_profile() -> Result<()> {
        let mut args = vec!["--development".to_string()];

        assert_eq!(take_profile(&mut args)?, BuildProfile::Debug);
        assert!(args.is_empty());
        Ok(())
    }

    #[test]
    fn release_flag_is_rejected_because_it_is_default() {
        let mut args = vec!["--release".to_string()];

        assert!(take_profile(&mut args).is_err());
    }

    #[test]
    fn duplicate_development_flags_are_rejected() {
        let mut args = vec!["--debug".to_string(), "--development".to_string()];

        assert!(take_profile(&mut args).is_err());
    }
}
