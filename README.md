# Tarn

A fast YouTube Music desktop client. Built with Tauri 2, Rust, and Svelte. Works with or without a Google account.

<p align="left"><img src="public/logo.png" width="72" alt="Tarn"></p>

![Home](docs/home.png)
![Playing](docs/playing.png)

## Install

Downloads are on the [releases](https://github.com/rexsystems/tarn/releases) page: AppImage, rpm and deb for Linux, NSIS/MSI for Windows, dmg for macOS.

Playback needs [ffmpeg](https://ffmpeg.org/) and [yt-dlp](https://github.com/yt-dlp/yt-dlp) on your `PATH`. The Linux packages pull them in as dependencies; on other platforms, install them yourself:

```
# Fedora
sudo dnf install ffmpeg yt-dlp

# Debian / Ubuntu
sudo apt install ffmpeg yt-dlp
```

## Build

Rust 1.77+, Node 20+, and (on Linux) the Tauri system libraries:

```
# Fedora
sudo dnf install webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel librsvg2-devel patchelf openssl-devel alsa-lib-devel

# Debian / Ubuntu
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev libasound2-dev
```

```
npm install
npm run tauri build
```

Packages land in `src-tauri/target/release/bundle/`. For a dev session: `npm run tauri dev`.

## License

[GPL-3.0](LICENSE)
