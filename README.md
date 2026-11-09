# avizo-rs

A Rust port of [avizo](https://github.com/regolith-linux/avizo), the
on-screen display of the [Regolith](https://regolith-desktop.com/) desktop on
sway: a small notification daemon, mainly for multimedia keys, that shows an
image and a progress bar (volume, microphone, brightness) as a layer-shell
surface. Regolith's avizo is a fork of
[heyjuvi/avizo](https://github.com/heyjuvi/avizo) (formerly misterdanb/avizo).

## Status

**Experimental port; not a drop-in replacement yet.**

- **A specific moment in time.** The port reproduces avizo as of
  [regolith-linux/avizo@7230b72](https://github.com/regolith-linux/avizo/commit/7230b7279a35dba77de786c90f02556bcc0276a1)
  (October 2026; program version 1.3, Debian package `regolith-avizo`
  0.1.6): the sync with heyjuvi/avizo `master` plus Regolith's fixes #6–#15.
  Both programs, `avizo-service` and `avizo-client`, are rewritten in Rust;
  the `volumectl` and `lightctl` helper scripts are unchanged.
- **Upstream fixes since then.** One fix was merged on regolith-linux/avizo
  `master` after `7230b72`, and it is ported here: #17 (`volumectl` accepts
  `-M` and `-h` after the command). There are no open pull requests on
  regolith-linux/avizo. On heyjuvi/avizo, the open pull requests #93–#99
  carry the same changes as Regolith's #7 and #10–#14 and the 1.3 version
  string, which are already in the port; #59 (an option to turn off the
  progress bar and show text) and #85 (support for another `lightctl`
  backend) are not ported. This list was made on 2026-11-09 and may not be
  complete: check upstream before relying on it.
- **Very deep testing is needed** before anyone considers this port as a
  replacement for the Vala avizo.
- **No deep code reviews yet.** The port has not yet been reviewed in depth
  by the major AI coding agents and code reviewers. Those reviews are
  planned before any replacement decision.
- **Upstreaming is undecided.** This repository is not a pull request to
  regolith-linux/avizo; whether, how and when any of it goes upstream will
  be decided later.

## What it ports

The two Vala programs, as one Cargo package with two binaries (gtk-rs 0.18:
GTK 3 and gtk-layer-shell):

- `avizo-service` owns `org.danb.avizo.service` on the session bus, with the
  same `Show` method and properties as the Vala service. It draws the
  display (background, border with any radius, image, block progress bar)
  in one layer-shell window per output, or only on a chosen monitor or on
  the focused output, fades it in and out, and hides it after the
  configured time. The built-in images are loaded at the output's scale, so
  they stay sharp on scaled outputs. A second instance exits when the name
  is already owned.
- `avizo-client` has the same options, help text, `avizo/config.ini`
  lookup in the XDG directories, number and colour parsing and exit codes
  as the Vala client. It sets the service's properties and calls `Show`;
  the session bus starts the service on demand through the D-Bus
  activation file.
- `build.rs` compiles the built-in images into the GResource bundle, as
  meson did, storing the PNGs as GdkPixdata (like `preprocess="to-pixdata"`)
  and the SVG sources as they are.

The D-Bus interface, the configuration file, the command lines, the
installed files and the Debian package name (`regolith-avizo`) are
unchanged.

## History

The branch is upstream's history up to `7230b72`, followed by:

1. `feat: port avizo-service and avizo-client from Vala to Rust`: the Cargo
   package, the Debian build with cargo, and the removal of the Vala
   sources and of the Regolith package-publishing workflows (`.github/`).
2. `fix(volumectl): accept -M and -h after the command (ports #17)`.
3. This README and the package version (`0.1.6+rs1`).

## Build

On Ubuntu 26.04, install the build dependencies from `debian/control`
(for example `sudo apt build-dep ./` from the checkout), then:

```sh
cargo build --release --locked      # fetches the locked crates
target/release/avizo-client --version
```

Offline builds: vendor the crates once, and cargo (and therefore the Debian
build) uses them:

```sh
mkdir -p .cargo
cargo vendor --locked vendor > .cargo/config.toml
```

`meson.build`, `data/meson.build`, `avizo.gresource.xml` and `ui/avizo.ui`
are left from the Vala build and are not used by the Rust build; meson no
longer builds the project.

## Test

```sh
RUSTFLAGS="-D warnings" cargo build --release --locked
cargo test --locked
cargo fmt --check
```

The crate has no unit tests yet: `cargo test` builds both binaries and runs
none. To try the display in a sway session:

```sh
target/release/avizo-service &
target/release/avizo-client --image-resource=volume_high --progress=0.5
```

## Package

```sh
dpkg-buildpackage -b -us -uc
```

builds `regolith-avizo_<version>_<arch>.deb` with the same files as the
Vala package: `avizo-service`, `avizo-client`, `volumectl`, `lightctl`,
`/etc/xdg/avizo/config.ini` and the D-Bus activation file. cargo needs
network access during the build unless the crates were vendored as above.

## Usage

Unchanged from the Vala program. From the original README:

### Configuration

Avizo can be configured using the configuration file and CLI options.

`avizo-client` looks for configuration file `avizo/config.ini` in the standard [XDG directories](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html) `XDG_CONFIG_HOME` (defaults to `~/.config`) and `XDG_CONFIG_DIRS` (defaults to `/etc/xdg`), in that order.
The first found file is used.
Missing configuration file is not an error.

The configuration file must be in INI format and should define keys in section named `default`.
Names of the configuration keys correspond to the CLI options (e.g. `block-height`).

Any configuration key can be overridden by corresponding CLI option (i.e. CLI options take precedence).

### Helper scripts

#### volumectl

Adjust the sound or mic volume and show Avizo notification.

**Requirements:**

* POSIX-sh compatible shell (e.g. Busybox ash, dash, ZSH, bash, …)
* common \*nix userland (BSD, Busybox or GNU)
* [pamixer](https://github.com/cdemoulins/pamixer)
* [pactl](https://www.freedesktop.org/wiki/Software/PulseAudio/Documentation/User/CLI/#pactl) for the `-p` option (currently playing sink/source) if pamixer is older than 1.6

#### lightctl

Adjust (display) brightness and show Avizo notification.

**Requirements:**

* POSIX-sh compatible shell (e.g. Busybox ash, dash, ZSH, bash, …)
* common \*nix userland (BSD, Busybox or GNU)
* [brightnessctl](https://github.com/Hummer12007/brightnessctl) or [light](https://gitlab.com/dpeukert/light)

### Sway config

```
bindsym XF86AudioRaiseVolume exec volumectl -u up
bindsym XF86AudioLowerVolume exec volumectl -u down
bindsym XF86AudioMute exec volumectl toggle-mute
bindsym XF86AudioMicMute exec volumectl -m toggle-mute

bindsym XF86MonBrightnessUp exec lightctl up
bindsym XF86MonBrightnessDown exec lightctl down

exec "avizo-service"
```

The `exec "avizo-service"` line is optional: the Debian package installs a
D-Bus activation file, so the session bus starts `avizo-service` on the
first notification (and again if it was stopped).

## License

GNU General Public License v3.0 or later, as the original program; see
[LICENSE](LICENSE). avizo was written by misterdanb and the avizo
contributors; the Regolith fork is maintained by the Regolith contributors.
