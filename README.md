<p align="right"><b>English</b> | <a href="README.ru.md">Русский</a></p>

# SC Desk

A SoundCloud app for Windows. No ads, light on your PC, with synced lyrics and Discord status.

**[Download the latest version](https://github.com/b1rkarokK/SC-Desktop/releases/latest)** - the `SC.Desk_x.y.z_x64-setup.exe` file.

Installs in a couple of seconds, no admin rights needed. You get a desktop shortcut and a Start menu entry. The app finds updates on its own and installs them with one click.

## What it does

- Likes: tracks, albums, playlists (yours and liked ones), artists you follow
- My Wave: an endless mix based on your taste. Pick a mood and keep tracks you already liked out of it
- Search all of SoundCloud, open track, artist and playlist pages
- Lyrics that light up in time with the song, sometimes word by word. There's a fullscreen mode you can tweak
- Listening history by day
- 10-band equalizer
- Themes: OLED, dark, light
- Discord status: what's playing, cover art and time left
- Lives in the tray, works with media keys, can start with Windows
- Switch headphones or speakers mid-song and the sound follows

No ads at all. Audio comes straight from SoundCloud, without the inserts the website plays.

## Signing in

Settings - "Sign in with SoundCloud". The regular SoundCloud login window opens, you can use email, Google, Facebook or Apple. Your login is kept in the Windows credential store and never leaves your PC.

## If SoundCloud doesn't load

By default the app goes online the same way your browser does, so if you use a VPN, it works through it. You can also set your own proxy in Settings - Network, there's a short guide and a "Check" button.

Tracks blocked in your country and SoundCloud Go+ tracks (they're DRM protected) are skipped.

## Building it yourself

You'll need Node.js 22+ and Rust.

```bash
npm ci
npm run tauri dev
npm run tauri build
```

## For the author: releasing a version

1. Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`.
2. Add what changed to `CHANGELOG.md`. Everyone sees this text in the update window.
3. `git tag v0.3.0 && git push --tags`. GitHub builds and publishes the rest.

---

SC Desk is an unofficial app and isn't affiliated with SoundCloud.
