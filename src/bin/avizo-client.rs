//! avizo-client: sets the display properties of avizo-service from the
//! command line and the configuration file, then asks it to show.

use std::ffi::OsString;
use std::io::Write as _;

use gio::prelude::*;

const BUS_NAME: &str = "org.danb.avizo.service";
const OBJECT_PATH: &str = "/org/danb/avizo/service";
const INTERFACE: &str = "org.danb.avizo.service";
const APPLICATION_ID: &str = "org.danb.avizo.client";
const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR")
);

const RESOURCES: [&str; 18] = [
    "volume_muted",
    "volume_muted_dark",
    "volume_low",
    "volume_low_dark",
    "volume_medium",
    "volume_medium_dark",
    "volume_high",
    "volume_high_dark",
    "mic_muted",
    "mic_muted_dark",
    "mic_unmuted",
    "mic_unmuted_dark",
    "brightness_low",
    "brightness_low_dark",
    "brightness_medium",
    "brightness_medium_dark",
    "brightness_high",
    "brightness_high_dark",
];

// ------------------------------------------------------------------ options

#[derive(Clone, Copy, PartialEq)]
enum Arg {
    None,
    Str,
    Int,
    Double,
}

#[derive(Clone, Copy)]
enum Key {
    Version,
    ListResources,
    ImageBaseDir,
    ImagePath,
    ImageResource,
    ImageOpacity,
    Progress,
    Width,
    Height,
    YOffset,
    XOffset,
    Padding,
    BorderRadius,
    BorderWidth,
    BlockHeight,
    BlockSpacing,
    BlockCount,
    FadeIn,
    FadeOut,
    Background,
    BorderColor,
    BarFgColor,
    BarBgColor,
    Time,
    Monitor,
    FocusedOutput,
}

struct Entry {
    long: &'static str,
    arg: Arg,
    key: Key,
    description: &'static str,
    arg_description: Option<&'static str>,
}

/// The options, in the order the help lists them and the configuration
/// file is read. Every long name is also a configuration key.
const ENTRIES: [Entry; 27] = [
    Entry {
        long: "version",
        arg: Arg::None,
        key: Key::Version,
        description: "Display version number",
        arg_description: None,
    },
    Entry {
        long: "list-resources",
        arg: Arg::None,
        key: Key::ListResources,
        description: "Lists the resource ids available",
        arg_description: None,
    },
    Entry {
        long: "image-base-dir",
        arg: Arg::Str,
        key: Key::ImageBaseDir,
        description: "The base directory to resolve relative image-path against (default is $XDG_DATA_HOME/avizo)",
        arg_description: Some("PATH"),
    },
    Entry {
        long: "image-path",
        arg: Arg::Str,
        key: Key::ImagePath,
        description: "Use the image specified by the path",
        arg_description: Some("PATH"),
    },
    Entry {
        long: "image-resource",
        arg: Arg::Str,
        key: Key::ImageResource,
        description: "Use the image specified by the image resource id",
        arg_description: Some("RESOURCE_ID"),
    },
    Entry {
        long: "image-opacity",
        arg: Arg::Double,
        key: Key::ImageOpacity,
        description: "Sets the image opacity, allowed values range from 0 (transparent) to 1.0 (opaque)",
        arg_description: Some("DOUBLE"),
    },
    Entry {
        long: "progress",
        arg: Arg::Double,
        key: Key::Progress,
        description: "Sets the progress in the notification, allowed values range from 0 to 1",
        arg_description: Some("DOUBLE"),
    },
    Entry {
        long: "width",
        arg: Arg::Int,
        key: Key::Width,
        description: "Sets the width of the notification",
        arg_description: Some("INT"),
    },
    Entry {
        long: "height",
        arg: Arg::Int,
        key: Key::Height,
        description: "Sets the height of the notification",
        arg_description: Some("INT"),
    },
    Entry {
        long: "y-offset",
        arg: Arg::Double,
        key: Key::YOffset,
        description: "Sets relative offset of the notification to the top of the screen, allowed values range from 0 (top) to 1.0 (bottom)",
        arg_description: Some("DOUBLE"),
    },
    Entry {
        long: "x-offset",
        arg: Arg::Double,
        key: Key::XOffset,
        description: "Sets relative offset of the notification to the left of the screen, allowed values range from 0 (left) to 1.0 (right)",
        arg_description: Some("DOUBLE"),
    },
    Entry {
        long: "padding",
        arg: Arg::Int,
        key: Key::Padding,
        description: "Sets the inner padding of the notification",
        arg_description: Some("INT"),
    },
    Entry {
        long: "border-radius",
        arg: Arg::Int,
        key: Key::BorderRadius,
        description: "Sets the border radius of the notification in px",
        arg_description: Some("INT"),
    },
    Entry {
        long: "border-width",
        arg: Arg::Int,
        key: Key::BorderWidth,
        description: "Sets the border width of the notification in px",
        arg_description: Some("INT"),
    },
    Entry {
        long: "block-height",
        arg: Arg::Int,
        key: Key::BlockHeight,
        description: "Sets the block height of the progress indicator",
        arg_description: Some("INT"),
    },
    Entry {
        long: "block-spacing",
        arg: Arg::Int,
        key: Key::BlockSpacing,
        description: "Sets the spacing between blocks in the progress indicator",
        arg_description: Some("INT"),
    },
    Entry {
        long: "block-count",
        arg: Arg::Int,
        key: Key::BlockCount,
        description: "Sets the amount of blocks in the progress indicator",
        arg_description: Some("INT"),
    },
    Entry {
        long: "fade-in",
        arg: Arg::Double,
        key: Key::FadeIn,
        description: "Sets the fade in animation duration in seconds",
        arg_description: Some("DOUBLE"),
    },
    Entry {
        long: "fade-out",
        arg: Arg::Double,
        key: Key::FadeOut,
        description: "Sets the fade out animation duration in seconds",
        arg_description: Some("DOUBLE"),
    },
    Entry {
        long: "background",
        arg: Arg::Str,
        key: Key::Background,
        description: "Sets the color of the notification background in format rgba([0, 255], [0, 255], [0, 255], [0, 1])",
        arg_description: Some("STRING"),
    },
    Entry {
        long: "border-color",
        arg: Arg::Str,
        key: Key::BorderColor,
        description: "Sets the color of the notification border in format rgba([0, 255], [0, 255], [0, 255], [0, 1])",
        arg_description: Some("STRING"),
    },
    Entry {
        long: "foreground",
        arg: Arg::Str,
        key: Key::BarFgColor,
        description: "Deprecated alias for --bar-fg-color",
        arg_description: Some("STRING"),
    },
    Entry {
        long: "bar-fg-color",
        arg: Arg::Str,
        key: Key::BarFgColor,
        description: "Sets the color of the filled bar blocks in format rgba([0, 255], [0, 255], [0, 255], [0, 1])",
        arg_description: Some("STRING"),
    },
    Entry {
        long: "bar-bg-color",
        arg: Arg::Str,
        key: Key::BarBgColor,
        description: "Sets the color of the unfilled bar blocks in format rgba([0, 255], [0, 255], [0, 255], [0, 1])",
        arg_description: Some("STRING"),
    },
    Entry {
        long: "time",
        arg: Arg::Double,
        key: Key::Time,
        description: "Sets the time to show the notification, default is 5",
        arg_description: Some("DOUBLE"),
    },
    Entry {
        long: "monitor",
        arg: Arg::Int,
        key: Key::Monitor,
        description: "Sets the monitor to show the notification on, default is all",
        arg_description: Some("INT"),
    },
    Entry {
        long: "focused-output",
        arg: Arg::None,
        key: Key::FocusedOutput,
        description: "Without --monitor, show the notification only on the focused output (Wayland)",
        arg_description: None,
    },
];

enum Value {
    Bool(bool),
    Str(String),
    Int(i32),
    Double(f64),
}

/// The client settings, starting from the program defaults.
struct Settings {
    show_version: bool,
    list_resources: bool,
    image_base_dir: String,
    image_path: String,
    image_resource: String,
    image_opacity: f64,
    progress: f64,
    width: i32,
    height: i32,
    y_offset: f64,
    x_offset: f64,
    padding: i32,
    border_radius: i32,
    border_width: i32,
    block_height: i32,
    block_spacing: i32,
    block_count: i32,
    fade_in: f64,
    fade_out: f64,
    background: String,
    border_color: String,
    bar_fg_color: String,
    bar_bg_color: String,
    time: f64,
    monitor: i32,
    focused_output: bool,
}

impl Settings {
    fn new() -> Settings {
        Settings {
            show_version: false,
            list_resources: false,
            image_base_dir: format!("{}/avizo", glib::user_data_dir().display()),
            image_path: String::new(),
            image_resource: "volume_muted".to_string(),
            image_opacity: 1.0,
            progress: 0.0,
            width: 248,
            height: 232,
            y_offset: 0.75,
            x_offset: 0.50,
            padding: 24,
            border_radius: 16,
            border_width: 1,
            block_height: 10,
            block_spacing: 2,
            block_count: 20,
            fade_in: 0.2,
            fade_out: 0.5,
            background: String::new(),
            border_color: String::new(),
            bar_fg_color: String::new(),
            bar_bg_color: String::new(),
            time: 5.0,
            monitor: -1,
            focused_output: false,
        }
    }

    fn set(&mut self, key: Key, value: Value) {
        match (key, value) {
            (Key::Version, Value::Bool(v)) => self.show_version = v,
            (Key::ListResources, Value::Bool(v)) => self.list_resources = v,
            (Key::FocusedOutput, Value::Bool(v)) => self.focused_output = v,
            (Key::ImageBaseDir, Value::Str(v)) => self.image_base_dir = v,
            (Key::ImagePath, Value::Str(v)) => self.image_path = v,
            (Key::ImageResource, Value::Str(v)) => self.image_resource = v,
            (Key::Background, Value::Str(v)) => self.background = v,
            (Key::BorderColor, Value::Str(v)) => self.border_color = v,
            (Key::BarFgColor, Value::Str(v)) => self.bar_fg_color = v,
            (Key::BarBgColor, Value::Str(v)) => self.bar_bg_color = v,
            (Key::Width, Value::Int(v)) => self.width = v,
            (Key::Height, Value::Int(v)) => self.height = v,
            (Key::Padding, Value::Int(v)) => self.padding = v,
            (Key::BorderRadius, Value::Int(v)) => self.border_radius = v,
            (Key::BorderWidth, Value::Int(v)) => self.border_width = v,
            (Key::BlockHeight, Value::Int(v)) => self.block_height = v,
            (Key::BlockSpacing, Value::Int(v)) => self.block_spacing = v,
            (Key::BlockCount, Value::Int(v)) => self.block_count = v,
            (Key::Monitor, Value::Int(v)) => self.monitor = v,
            (Key::ImageOpacity, Value::Double(v)) => self.image_opacity = v,
            (Key::Progress, Value::Double(v)) => self.progress = v,
            (Key::YOffset, Value::Double(v)) => self.y_offset = v,
            (Key::XOffset, Value::Double(v)) => self.x_offset = v,
            (Key::FadeIn, Value::Double(v)) => self.fade_in = v,
            (Key::FadeOut, Value::Double(v)) => self.fade_out = v,
            (Key::Time, Value::Double(v)) => self.time = v,
            _ => {}
        }
    }
}

// -------------------------------------------------------- number parsing

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `strtol(text, &end, 0)`: the value (clamped to the range of a 64-bit
/// long), whether the whole text was consumed, and whether it overflowed.
fn strtol(text: &[u8]) -> (i64, bool, bool) {
    let mut i = 0;
    while i < text.len() && is_c_space(text[i]) {
        i += 1;
    }
    let mut negative = false;
    if i < text.len() && (text[i] == b'+' || text[i] == b'-') {
        negative = text[i] == b'-';
        i += 1;
    }
    let base: u32 = if text.get(i) == Some(&b'0')
        && matches!(text.get(i + 1), Some(b'x') | Some(b'X'))
        && text.get(i + 2).is_some_and(|b| b.is_ascii_hexdigit())
    {
        i += 2;
        16
    } else if text.get(i) == Some(&b'0') {
        8
    } else {
        10
    };
    let start = i;
    let mut magnitude: u128 = 0;
    let mut overflow = false;
    while let Some(digit) = text.get(i).and_then(|&b| (b as char).to_digit(base)) {
        magnitude = magnitude * base as u128 + digit as u128;
        if magnitude > (1u128 << 64) {
            magnitude = 1u128 << 64;
            overflow = true;
        }
        i += 1;
    }
    if i == start {
        // No conversion: end points at the start of the text.
        return (0, text.is_empty(), false);
    }
    let value = if negative {
        if magnitude > 1u128 << 63 {
            overflow = true;
            i64::MIN
        } else {
            (magnitude as i128).wrapping_neg() as i64
        }
    } else if magnitude > i64::MAX as u128 {
        overflow = true;
        i64::MAX
    } else {
        magnitude as i64
    };
    (value, i == text.len(), overflow)
}

/// `strtod(text, &end)` in the C locale (as `g_ascii_strtod`): the value,
/// whether the whole text was consumed, and whether it is out of range.
fn strtod(text: &[u8]) -> (f64, bool, bool) {
    let mut i = 0;
    while i < text.len() && is_c_space(text[i]) {
        i += 1;
    }
    let mut negative = false;
    if i < text.len() && (text[i] == b'+' || text[i] == b'-') {
        negative = text[i] == b'-';
        i += 1;
    }
    let sign = if negative { -1.0 } else { 1.0 };
    let rest = &text[i..];
    let lower = |n: usize| -> Vec<u8> {
        rest.iter()
            .take(n)
            .map(|b| b.to_ascii_lowercase())
            .collect()
    };

    // Infinity and NaN.
    if lower(8) == b"infinity" {
        return (sign * f64::INFINITY, i + 8 == text.len(), false);
    }
    if lower(3) == b"inf" {
        return (sign * f64::INFINITY, i + 3 == text.len(), false);
    }
    if lower(3) == b"nan" {
        let mut end = i + 3;
        if text.get(end) == Some(&b'(') {
            let mut j = end + 1;
            while text
                .get(j)
                .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
            {
                j += 1;
            }
            if text.get(j) == Some(&b')') {
                end = j + 1;
            }
        }
        return (f64::NAN, end == text.len(), false);
    }

    // Hexadecimal floating point.
    if rest.len() > 2
        && rest[0] == b'0'
        && (rest[1] == b'x' || rest[1] == b'X')
        && (rest[2].is_ascii_hexdigit()
            || (rest[2] == b'.' && rest.get(3).is_some_and(|b| b.is_ascii_hexdigit())))
    {
        let mut j = i + 2;
        let mut mantissa: u128 = 0;
        let mut exponent: i64 = 0;
        let mut sticky = false;
        let mut seen_point = false;
        let mut nonzero = false;
        loop {
            match text.get(j) {
                Some(b'.') if !seen_point => seen_point = true,
                Some(b) if b.is_ascii_hexdigit() => {
                    let digit = (*b as char).to_digit(16).unwrap_or(0) as u128;
                    nonzero |= digit != 0;
                    if mantissa >> 116 == 0 {
                        mantissa = mantissa << 4 | digit;
                        if seen_point {
                            exponent -= 4;
                        }
                    } else {
                        sticky |= digit != 0;
                        if !seen_point {
                            exponent += 4;
                        }
                    }
                }
                _ => break,
            }
            j += 1;
        }
        if let Some((value, end)) = binary_exponent(text, j) {
            exponent = exponent.saturating_add(value);
            j = end;
        }
        if sticky {
            mantissa |= 1;
        }
        let value = scale_binary(mantissa, exponent);
        let range = value.is_infinite() || (nonzero && (value == 0.0 || value.is_subnormal()));
        return (sign * value, j == text.len(), range);
    }

    // Decimal.
    let mut j = i;
    let mut digits = String::new();
    let mut seen_point = false;
    let mut any_digit = false;
    loop {
        match text.get(j) {
            Some(b'.') if !seen_point => {
                seen_point = true;
                digits.push('.');
            }
            Some(b) if b.is_ascii_digit() => {
                any_digit = true;
                digits.push(*b as char);
            }
            _ => break,
        }
        j += 1;
    }
    if !any_digit {
        return (0.0, text.is_empty(), false);
    }
    if let Some(&e) = text.get(j) {
        if e == b'e' || e == b'E' {
            let mut k = j + 1;
            let mut exponent = String::from("e");
            if let Some(&s) = text.get(k) {
                if s == b'+' || s == b'-' {
                    exponent.push(s as char);
                    k += 1;
                }
            }
            if text.get(k).is_some_and(|b| b.is_ascii_digit()) {
                while let Some(&d) = text.get(k).filter(|b| b.is_ascii_digit()) {
                    exponent.push(d as char);
                    k += 1;
                }
                digits.push_str(&exponent);
                j = k;
            }
        }
    }
    let value: f64 = digits.parse().unwrap_or(0.0);
    let nonzero = digits
        .split(['e', 'E'])
        .next()
        .is_some_and(|m| m.bytes().any(|b| (b'1'..=b'9').contains(&b)));
    let range = value.is_infinite() || (nonzero && (value == 0.0 || value.is_subnormal()));
    (sign * value, j == text.len(), range)
}

/// Parses a `p[+-]digits` binary exponent starting at `at`.
fn binary_exponent(text: &[u8], at: usize) -> Option<(i64, usize)> {
    if !matches!(text.get(at), Some(b'p') | Some(b'P')) {
        return None;
    }
    let mut k = at + 1;
    let mut negative = false;
    if let Some(&s) = text.get(k) {
        if s == b'+' || s == b'-' {
            negative = s == b'-';
            k += 1;
        }
    }
    if !text.get(k).is_some_and(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut value: i64 = 0;
    while let Some(&d) = text.get(k).filter(|b| b.is_ascii_digit()) {
        value = value.saturating_mul(10).saturating_add((d - b'0') as i64);
        k += 1;
    }
    Some((if negative { -value } else { value }, k))
}

/// `mantissa * 2^exponent`, rounded to the nearest double.
fn scale_binary(mantissa: u128, exponent: i64) -> f64 {
    if mantissa == 0 {
        return 0.0;
    }
    let mut value = mantissa as f64;
    let mut exponent = exponent.clamp(-4000, 4000);
    while exponent > 0 {
        let step = exponent.min(1000);
        value *= 2f64.powi(step as i32);
        exponent -= step;
    }
    while exponent < 0 {
        let step = (-exponent).min(1000);
        value /= 2f64.powi(step as i32);
        exponent += step;
    }
    value
}

// ------------------------------------------------------ GOption semantics

fn parse_value(arg: Arg, option: &str, value: &str) -> Result<Value, String> {
    match arg {
        Arg::None => Ok(Value::Bool(true)),
        Arg::Str => Ok(Value::Str(value.to_string())),
        Arg::Int => {
            let (number, complete, overflow) = strtol(value.as_bytes());
            if value.is_empty() || !complete {
                return Err(format!("Cannot parse integer value “{value}” for {option}"));
            }
            if overflow || i32::try_from(number).is_err() {
                return Err(format!("Integer value “{value}” for {option} out of range"));
            }
            Ok(Value::Int(number as i32))
        }
        Arg::Double => {
            let (number, complete, range) = strtod(value.as_bytes());
            if value.is_empty() || !complete {
                return Err(format!("Cannot parse double value “{value}” for {option}"));
            }
            if range {
                return Err(format!("Double value “{value}” for {option} out of range"));
            }
            Ok(Value::Double(number))
        }
    }
}

fn help_text() -> String {
    let program = glib::prgname()
        .map(|p| p.to_string())
        .unwrap_or_else(|| "avizo-client".to_string());
    let widths = ENTRIES
        .iter()
        .map(|e| e.long.chars().count() + e.arg_description.map_or(0, |d| 1 + d.chars().count()));
    // The help entry has a short name: "-h, " adds four columns.
    let max_length = widths
        .chain(std::iter::once("help".len() + 4))
        .max()
        .unwrap_or(0)
        + 4;
    let line = |option: String, description: &str| -> String {
        let pad = (max_length + 4).saturating_sub(option.chars().count());
        format!("{option}{:pad$} {description}\n", "")
    };
    let mut text = format!("Usage:\n  {program} [OPTION…] - Run avizo-client\n\n");
    text.push_str("Help Options:\n");
    text.push_str(&line("  -h, --help".to_string(), "Show help options"));
    text.push_str("\nApplication Options:\n");
    for entry in &ENTRIES {
        let mut option = format!("  --{}", entry.long);
        if let Some(arg) = entry.arg_description {
            option.push('=');
            option.push_str(arg);
        }
        text.push_str(&line(option, entry.description));
    }
    text.push('\n');
    text
}

fn print_help_and_exit() -> ! {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(help_text().as_bytes());
    let _ = stdout.flush();
    std::process::exit(0);
}

/// Parses the arguments as GLib's option parser does for these entries:
/// options and positional arguments may mix, `--` ends the options, a
/// value is given with `=` or as the next argument, and `--help`, `-h`
/// and `-?` print the help and exit.
fn parse_options(args: &[String], settings: &mut Settings) -> Result<(), String> {
    let mut changes: Vec<(Key, Value)> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        let bytes = arg.as_bytes();
        if bytes.len() < 2 || bytes[0] != b'-' {
            i += 1;
            continue;
        }
        if bytes[1] == b'-' {
            let name = &arg[2..];
            if name.is_empty() {
                break;
            }
            if name == "help" || name == "help-all" {
                print_help_and_exit();
            }
            let mut parsed = false;
            for entry in &ENTRIES {
                if entry.arg == Arg::None && name == entry.long {
                    changes.push((entry.key, Value::Bool(true)));
                    parsed = true;
                    break;
                }
                let len = entry.long.len();
                if name.len() >= len
                    && &name.as_bytes()[..len] == entry.long.as_bytes()
                    && (name.len() == len || name.as_bytes()[len] == b'=')
                {
                    let option = format!("--{}", entry.long);
                    let value = if name.len() > len {
                        name[len + 1..].to_string()
                    } else if i + 1 < args.len() {
                        i += 1;
                        args[i].clone()
                    } else {
                        return Err(format!("Missing argument for {option}"));
                    };
                    changes.push((entry.key, parse_value(entry.arg, &option, &value)?));
                    parsed = true;
                    break;
                }
            }
            if !parsed {
                return Err(format!("Unknown option {arg}"));
            }
        } else {
            // No entry has a short name: only the help options exist, and
            // the first other letter is an unknown option.
            if matches!(arg[1..].chars().next(), Some('?' | 'h')) {
                print_help_and_exit();
            }
            return Err(format!("Unknown option {arg}"));
        }
        i += 1;
    }
    for (key, value) in changes {
        settings.set(key, value);
    }
    Ok(())
}

// ------------------------------------------------------------ configuration

/// Reads the first group of the first `avizo/config.ini` found in the user
/// and system configuration directories. Keys are the long option names.
fn load_config(settings: &mut Settings) -> Result<(), glib::Error> {
    let mut dirs = vec![glib::user_config_dir()];
    dirs.extend(glib::system_config_dirs());
    let conf = glib::KeyFile::new();
    conf.load_from_dirs("avizo/config.ini", &dirs, glib::KeyFileFlags::NONE)?;
    let Some(group) = conf.start_group() else {
        return Ok(());
    };
    for entry in &ENTRIES {
        if !conf.has_key(&group, entry.long)? {
            continue;
        }
        let value = match entry.arg {
            Arg::Double => Value::Double(conf.double(&group, entry.long)?),
            Arg::Int => Value::Int(conf.integer(&group, entry.long)?),
            Arg::Str => Value::Str(conf.string(&group, entry.long)?.to_string()),
            Arg::None => Value::Bool(conf.boolean(&group, entry.long)?),
        };
        settings.set(entry.key, value);
    }
    Ok(())
}

/// `g_canonicalize_filename()`: makes `filename` absolute against
/// `relative_to` and removes `.`, `..` and repeated separators.
fn canonicalize_filename(filename: &str, relative_to: &str) -> String {
    let joined = if filename.starts_with('/') {
        filename.to_string()
    } else {
        let base = relative_to.trim_end_matches('/');
        let name = filename.trim_start_matches('/');
        if base.is_empty() && relative_to.starts_with('/') {
            format!("{relative_to}{name}")
        } else {
            format!("{base}/{name}")
        }
    };
    let leading = joined.bytes().take_while(|&b| b == b'/').count();
    if leading == 0 {
        return format!("/{}", filename.trim_start_matches('/'));
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in joined[leading..].split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let root = if leading == 2 { "//" } else { "/" };
    format!("{root}{}", parts.join("/"))
}

/// Parses a colour as GDK does; an invalid colour ends the client the way
/// a GLib error does.
fn parse_rgba(value: &str) -> gdk::RGBA {
    match gdk::RGBA::parse(value) {
        Ok(color) => color,
        Err(_) => {
            let now = glib::DateTime::now_local()
                .ok()
                .and_then(|t| {
                    let ms = t.microsecond() / 1000;
                    t.format("%H:%M:%S").ok().map(|s| format!("{s}.{ms:03}"))
                })
                .unwrap_or_default();
            let program = glib::prgname()
                .map(|p| p.to_string())
                .unwrap_or_else(|| "avizo-client".to_string());
            eprint!(
                "\n** ({program}:{}): ERROR **: {now}: Invalid RGBA color value: {value}\n",
                std::process::id()
            );
            std::process::abort();
        }
    }
}

fn rgba_variant(color: &gdk::RGBA) -> glib::Variant {
    (color.red(), color.green(), color.blue(), color.alpha()).to_variant()
}

// --------------------------------------------------------------------- run

struct Service {
    proxy: gio::DBusProxy,
}

impl Service {
    fn connect() -> Result<Service, glib::Error> {
        let connection = gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>)?;
        let proxy = gio::DBusProxy::new_sync(
            &connection,
            gio::DBusProxyFlags::NONE,
            None,
            Some(BUS_NAME),
            OBJECT_PATH,
            INTERFACE,
            None::<&gio::Cancellable>,
        )?;
        Ok(Service { proxy })
    }

    /// Sets one property; like a property assignment on a D-Bus proxy,
    /// a failure is ignored.
    fn set(&self, name: &str, value: glib::Variant) {
        let arguments = (INTERFACE, name, value).to_variant();
        let _ = self.proxy.call_sync(
            "org.freedesktop.DBus.Properties.Set",
            Some(&arguments),
            gio::DBusCallFlags::NONE,
            -1,
            None::<&gio::Cancellable>,
        );
    }

    fn show(&self, seconds: f64) -> Result<(), glib::Error> {
        self.proxy
            .connection()
            .call_sync(
                Some(BUS_NAME),
                OBJECT_PATH,
                INTERFACE,
                "Show",
                Some(&(seconds,).to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                self.proxy.default_timeout(),
                None::<&gio::Cancellable>,
            )
            .map(|_| ())
    }
}

fn command_line(service: &Service, arguments: Vec<OsString>) -> i32 {
    let mut settings = Settings::new();
    if let Err(error) = load_config(&mut settings) {
        if !error.matches(glib::KeyFileError::NotFound) {
            eprint!(
                "avizo: Failed to load configuration file: {}\n",
                error.message()
            );
            return 1;
        }
    }

    let args: Vec<String> = arguments
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if let Err(message) = parse_options(&args, &mut settings) {
        let program = args.first().map(String::as_str).unwrap_or("avizo-client");
        eprint!("avizo: {message}\n");
        eprint!("Run '{program} --help' to see a full list of available command line options.\n");
        return 1;
    }

    if settings.show_version {
        print!("avizo-client {VERSION}\n");
        let _ = std::io::stdout().flush();
        return 0;
    }

    if settings.list_resources {
        let mut text = String::from("Available resources:\n");
        for resource in RESOURCES {
            text.push_str(&format!("  {resource}\n"));
        }
        print!("{text}");
        let _ = std::io::stdout().flush();
        return 0;
    }

    if !settings.image_path.is_empty() {
        let path = canonicalize_filename(&settings.image_path, &settings.image_base_dir);
        service.set("ImagePath", path.to_variant());
    } else {
        service.set("ImageResource", settings.image_resource.to_variant());
    }

    service.set("ImageOpacity", settings.image_opacity.to_variant());
    service.set("Progress", settings.progress.to_variant());
    service.set("Width", settings.width.to_variant());
    service.set("Height", settings.height.to_variant());
    service.set("Padding", settings.padding.to_variant());
    service.set("YOffset", settings.y_offset.to_variant());
    service.set("XOffset", settings.x_offset.to_variant());
    service.set("BorderRadius", settings.border_radius.to_variant());
    service.set("BorderWidth", settings.border_width.to_variant());
    service.set("BlockHeight", settings.block_height.to_variant());
    service.set("BlockSpacing", settings.block_spacing.to_variant());
    service.set("BlockCount", settings.block_count.to_variant());
    service.set("FadeIn", settings.fade_in.to_variant());
    service.set("FadeOut", settings.fade_out.to_variant());
    service.set("Monitor", settings.monitor.to_variant());
    service.set("FocusedOutput", settings.focused_output.to_variant());

    if !settings.background.is_empty() {
        let color = parse_rgba(&settings.background);
        service.set("Background", rgba_variant(&color));
        if settings.bar_bg_color.is_empty() {
            // The unfilled blocks default to the background at 2/3 brightness.
            let bar = (
                color.red() / 1.5,
                color.green() / 1.5,
                color.blue() / 1.5,
                color.alpha(),
            );
            service.set("BarBgColor", bar.to_variant());
        }
    }
    if !settings.border_color.is_empty() {
        service.set(
            "BorderColor",
            rgba_variant(&parse_rgba(&settings.border_color)),
        );
    }
    if !settings.bar_bg_color.is_empty() {
        service.set(
            "BarBgColor",
            rgba_variant(&parse_rgba(&settings.bar_bg_color)),
        );
    }
    if !settings.bar_fg_color.is_empty() {
        service.set(
            "BarFgColor",
            rgba_variant(&parse_rgba(&settings.bar_fg_color)),
        );
    }

    if let Err(error) = service.show(settings.time) {
        if error.domain() == glib::Quark::from_str("g-dbus-error-quark") {
            eprint!("avizo: {}\n", error.message());
            eprint!("Failed to acquire bus, is avizo-service running?\n");
            return 1;
        }
        eprint!("avizo: uncaught error: {}\n", error.message());
    }
    0
}

fn main() {
    // The service is looked up first; a bus-activatable service is started
    // by this lookup.
    let service = match Service::connect() {
        Ok(service) => service,
        Err(error) => {
            eprint!("avizo: {}\n", error.message());
            std::process::exit(0);
        }
    };

    let app = gio::Application::new(
        Some(APPLICATION_ID),
        gio::ApplicationFlags::HANDLES_COMMAND_LINE,
    );
    app.connect_command_line(move |_, command| command_line(&service, command.arguments()));
    let args: Vec<String> = std::env::args_os()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // The exit status is 0 whatever the outcome, as the client has always
    // done; only an invalid colour aborts.
    let _ = app.run_with_args(&args);
    std::process::exit(0);
}
