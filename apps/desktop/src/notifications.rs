//! Native Linux notifications. Arguments are never evaluated by a shell.
use bufonidae_core::config::{Config, DesktopOptions, SoundMode};
use std::{process::Stdio, time::Duration};

fn variant(text: &str) -> String {
    format!(
        "'{}'",
        text.replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}

pub async fn deliver(
    options: &DesktopOptions,
    title: &str,
    body: &str,
    failed: bool,
) -> Result<(), String> {
    options.validate().map_err(|error| error.to_string())?;
    let body = body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let mut command = tokio::process::Command::new("gdbus");
    command
        .args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.Notifications",
            "--object-path",
            "/org/freedesktop/Notifications",
            "--method",
            "org.freedesktop.Notifications.Notify",
        ])
        .args([
            variant("Bufonidae"),
            "0".into(),
            variant("bufonidae"),
            variant(title),
            variant(&body),
            "[]".into(),
            "{}".into(),
            (u32::from(options.notification_seconds) * 1000).to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(3), command.output())
        .await
        .map_err(|_| "Desktop notification service timed out.".to_owned())?
        .map_err(|error| format!("Could not start gdbus: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Desktop notification service rejected delivery: {}",
            bufonidae_rclone::escape_control(&String::from_utf8_lossy(&output.stderr))
        ));
    }
    if options.sound == SoundMode::All || (failed && options.sound == SoundMode::Errors) {
        let path = if failed {
            "/usr/share/sounds/freedesktop/stereo/dialog-error.oga"
        } else {
            "/usr/share/sounds/freedesktop/stereo/complete.oga"
        };
        let mut sound = tokio::process::Command::new("paplay");
        sound
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let status = tokio::time::timeout(Duration::from_secs(5), sound.status())
            .await
            .map_err(|_| "Notification accepted, but sound playback timed out.".to_owned())?
            .map_err(|error| {
                format!("Notification accepted, but paplay could not start: {error}")
            })?;
        if !status.success() {
            return Err("Notification accepted, but sound playback failed.".into());
        }
    }
    Ok(())
}

pub async fn announce(failed: bool, cancelled: bool, detail: &str) -> Result<(), String> {
    let config = Config::load(Config::default_path().map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let options = config.desktop();
    let enabled = if cancelled {
        options.notify_cancel
    } else if failed {
        options.notify_failure
    } else {
        options.notify_success
    };
    if !enabled {
        return Ok(());
    }
    let title = if cancelled {
        "Transfer cancelled"
    } else if failed {
        "Transfer needs attention"
    } else {
        "Transfer completed"
    };
    let body = if options.notification_names {
        bufonidae_rclone::escape_control(detail)
    } else {
        "Open Transfers in Bufonidae for the result.".into()
    };
    deliver(options, title, &body, failed).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_strings_remain_single_variant_arguments() {
        assert_eq!(variant("a'b\\c\n"), "'a\\'b\\\\c\\n'");
        assert_eq!(
            variant("$(touch /tmp/no); <file>"),
            "'$(touch /tmp/no); <file>'"
        );
    }
}
