# Changes

English twin of `CHANGELOG.md`: same versions, same headings `## [x.y.z] - date`.
The release notes carry both; the "Update available" window shows the app's language.

## [0.9.2] - 2026-10-13

- "Categories": noticeably more genres. Metal, jazz, classical, country, folk, Latin, reggae, reggaeton, dancehall, disco, synthwave and world music joined the genres (these are real SoundCloud trends).
- A new "Phonk and funk" block: phonk is split by kind (general, drift, aggressive, dark, Memphis and old-school, Brazilian, phonk house and wave), and funk stands on its own so the two are not mixed up.
- A new "Underground and strange" block: hyperpop, rage, plugg, drill, jerk, emo and emo rap, hardtekk, hardstyle, vaporwave, lo-fi, glitch, dark ambient, shoegaze, post-punk, R&B, Slowed + reverb, Speed up and nightcore.
- "Anti-music" is a pick of a scene, not of one artist: dekma is the sample, then come those who record with him and around him, and the taste model puts the tracks in order by how close they sound. No more than three tracks from one artist.

## [0.9.1] - 2026-10-13

- Slowed and Sped up: the lyrics and the progress bar no longer drift when the speed changes. The position used to be counted in output time, now it is counted in the track's own time.
- For sped-up and slowed versions (speed up, slowed, nightcore) the original's timed lyrics are used and their timings are recalculated for the tempo.
- Under the lyrics the "-" and "+" buttons move the text by a quarter of a second if it is early or late; the shift is remembered for each track. Matching the lyrics to the sound can be turned on and off in Settings.
- Friends see what you listen to (only they, can be turned off in Settings), and you see what they listen to: "Profile - Friends" and the "Rooms" page. You cannot join a person: only their open room, or by invitation. Rooms with a password are no longer shown to friends.
- Friend requests: protection from a flood of fake ones and a hint that you follow the person. Friends and shared playlists start right after you sign in, not after a restart.
- First start without signing in: a clear "Sign in with SoundCloud" button on the home page and in "Likes" instead of an empty page and red errors.
- Equalizer: the note about SoundCloud's player takes the whole width of the window. The "new" dot of shared playlists goes out once you have looked at everything.
- "My Wave" takes into account more tracks you did not like.
- Hotkeys show the "," and "." signs instead of words. The switches in Settings have names for screen readers. Old lyrics records in the database no longer cause an error.

## [0.9.0] - 2026-10-12

- Friends: the app now has its own friends, not SoundCloud followers. Profile - "Friends": find a person, send a request, they accept and you are friends. You can invite friends to a room (a password included) and to a shared playlist: the invitation arrives in the app as a notification and waits in the tab. Everything is encrypted with the pair's key, an invitation cannot be forged. A dot on the avatar shows a waiting request or invitation. Friends' open rooms show in "Rooms".
- A person's profile: artists and friends get "Likes" and "Liked playlists" tabs (their playlists were there already), and friends get a "Friend" mark.
- Own themes: Settings - "Theme" - "Custom theme". Four colours, a light or dark base, extra colours, corner radius, a background picture (dimming, blur, panel opacity) and your own CSS. A theme code can be copied and pasted. A theme is light: the picture is scaled once, there are no filters or animations.
- Rooms: the room code is shown (it copies), the current track and the queue have a like heart, the host removes tracks from the queue and lets removed members back ("Removed" on the right, "Let back"). The people count in other rooms no longer jumps when a track changes. An "Invite a friend" button.
- Shared playlists: an "Add track" button (search) and "To a shared playlist" in a track's menu. A friend's invitation reaches them in the app.
- Hotkeys Slowed (Ctrl+Alt+,) and Sped up (Ctrl+Alt+.): pressing again returns the normal speed.
- Volume: the left part of the slider is no longer "dead", the sound goes only at the very edge, where the icon turns to "muted".
- Your own uploaded tracks can be liked (the "My tracks" tab).
- The equalizer button lights up at once at start if it was on. Primary buttons no longer turn black on hover.
- Discord: the "Join" button for an open room is on by default (turn it off in Settings). The link from Discord opens the app, or the download page if it is not there.
- Proxy: a few seconds after start the app reconnects through the proxy (before, after a restart you sometimes had to press "Apply").

## [0.8.1] - 2026-10-12

- Output device picker: a capsule with the device name in the title bar. The list has "Like the system" and all devices, the choice is remembered, and if the device disappears the system one plays. The list opens instantly.
- After someone is kicked from a room, new members join without trouble, and the kicked one cannot come back or create a room of their own.
- The room badge and the device capsule follow the dark, light and OLED themes.
- The app now has a website: https://b1rkarokk.github.io/SC-Desktop/ (Russian and English).

## [0.8.0] - 2026-10-12

- "Rooms": listen together, one track at everyone's place at once. Audio and chat are not sent, only commands. A room stays in "My rooms" even when nobody is in it: whoever comes first becomes the host, and when the host leaves the music goes on and the next person takes over. Pause, seek, next and previous from the host work for everyone, and a track a member clicks goes to the room's queue. On every tab a "You are in a room" badge in the title bar offers quick actions.
- "Shared playlists": one list for everybody, every track shows who added it, and there is a history of changes. Roles (owner, editor, viewer) and access: private, by link, by invitation, open. Appearance: your own picture, a colour or the artwork of the tracks. "Play together" creates a room with the playlist's queue.
- `scdesk://` links open a room or a shared playlist right in the app (it asks before joining).
- Rooms and shared playlists use a free sync server that is already built in. Everything is encrypted on your devices, the server only sees ciphertext.
- "My Wave" takes into account how the tracks you listened to sound (tempo, brightness, energy), and a wave from a playlist is now closer to the playlist itself.
- Lyrics match the sound better: the clock corrects itself in both directions, the version of the text is chosen by the track's length, and a note warns when it differs a lot.
- The Discord status shows where you are in the app ("Browsing likes", "On the home page") and when you are in a room. The "Join" button for an open room is turned on in the settings.
- The equalizer is remembered separately for every output device, the sliders no longer get stuck. The equalizer and the queue close with a click elsewhere.
- Auto-pause during a call and during a full-screen game (off by default, turned on in the settings).
- A backup of likes, history and settings, "Copy log", search of your likes by a line of a song (Ctrl+K - "By lyrics") and "Download all" on a playlist page for listening offline.
- The Genius check no longer pops up over a game.

## [0.7.1] - 2026-10-12

- "My Wave" starts faster: the shortest of the first tracks plays first, so there is almost no waiting for the start.
- The equalizer window now says so when a track plays in SoundCloud's own player: the equalizer and effects can't reach such tracks.

## [0.7.0] - 2026-10-12

- Your own hotkeys: Settings - "Hotkeys". Pause, next and previous track, like, volume up and down, mini player. Click a key in the list and press the new combination. The "Work in games" switch catches them over a full-screen game.
- Quick search Ctrl+K: tracks, artists, playlists and your likes in one box. Enter plays the track and a wave from it, Ctrl+Enter adds it to the queue, Esc closes.
- Visualizer in full screen: bars, mirror, line, waves and a ring around the cover. Pick the color: accent, from the artwork or white. Chosen in the "View" panel.
- Share a track: "Share…" in the track menu and a button in full screen. The link can be copied right at the second you are on.
- "My Wave" got smarter: a small model learns from your likes, skips and finished tracks right on your computer. It looks at language, genre, time of day and weekday, and tries new things more often in "Fresh" mode.
- Lyrics are found even more often: when nothing turns up for the artist, the song is searched on Genius by its title alone (re-uploads and covers).
- The update notes now come in two languages, the window shows the language of the interface.

## [0.6.1] - 2026-10-11

- Windows media panel: the track, its cover and the buttons in the volume flyout, on the lock screen and in quick settings.
- Lyrics are found with word-by-word highlighting again, plus one more lyrics source (Deezer).
- With slowed and sped up, the lyrics and the progress bar keep up with the sound.
- The mini player has a like button and an opacity setting from 10% to 100%.
- Clearing the history now asks first.

## [0.6.0] - 2026-10-10

- Mini player: a small capsule on top of other windows with the current lyric line. Hover it for the seek bar, every button, volume and the queue. Opens from a button in the player or from the tray, double click brings the big window back. While it is open the big window uses no memory.
- In full-screen games and videos the mini player stays visible but out of the mouse's way: clicks go through it.
- Stats: hours of music over a week, a month, a year and all time, favorite tracks and artists, genres and when you listen.
- Import playlists from Yandex Music and Spotify: Likes - Playlists - "Import". Songs are matched on SoundCloud strictly, without remixes or covers, and the ones not found are listed.
- Full screen now covers the whole window in the gray and light themes too, with minimize, maximize and close at the top right.

## [0.5.2] - 2026-10-10

- The volume slider works again: it no longer shrinks while dragged.

## [0.5.1] - 2026-10-10

- Fixed the volume slider shrinking to a dot next to the new player buttons.

## [0.5.0] - 2026-10-10

- "My Wave" learns from you: a track played to the end brings more like it, one skipped in the first seconds makes that artist rarer. Skipping a liked song doesn't count.
- Wave from a playlist or album, and a wave from a track or artist no longer drifts into the regular one or stops.
- Smart Shuffle like in Spotify: press "shuffle" once more and similar recommendations play between the list's tracks.
- Queue: a new button in the player, tracks can be dragged, removed and played with a click.
- Playlists: "Add to playlist" in the track menu, a new playlist in two clicks, reorder and remove tracks in your own playlists.
- Upload your tracks to SoundCloud: Profile - "My tracks" - "Upload", with artwork, genre and private access.
- Effects in the equalizer window: slowed and sped up, reverb, crossfade between tracks and loudness leveling.
- Sleep timer: 15-60 minutes, a custom time or "after this track", the sound fades out gently.
- Windows notifications about new tracks from artists you follow (can be turned off in Settings).
- English interface: Settings - "Язык / Language".
- The captcha on likes shows up less and never in the middle of things: the like waits in the queue and a dot on the avatar says "verify".
- Slowed, sped up and other versions show the original song's lyrics when they have none of their own.
- Small fixes.

## [0.4.1] - 2026-10-10

- Fixed "SoundCloud rejected the token (401)": the app renews the sign-in and client_id by itself, no need to sign in again.
- The right-click menu no longer closes by itself while a song plays.

## [0.4.0] - 2026-10-10

- New "Home" section: your mixes, "Made for you", stations, trends and SoundCloud playlists, plus a feed of new tracks and reposts from the people you follow.
- "Categories": picks found only in SC Desk (underground radar, forgotten likes, a year ago, on repeat), charts by genre, eras, Russian music and AI music. Tiles with the covers of the top tracks.
- Real charts: today in Russia and worldwide by Yandex Music, hits of the week, month and year, every year since 2001 (Russia and worldwide). Songs are matched strictly, without remixes, covers or other people's versions.
- 30-second Go+ previews now play in full: the app finds a full upload of the same recording and checks it by sound. The lyrics shift if that upload has a longer intro.
- The next track is prepared in advance and starts at once. Protected tracks start fast too ("Instant start of protected tracks").
- Lyrics are found more often: for covers, re-uploads and titles in Latin letters. Genius works again; for its Cloudflare check a window with a checkbox shows up.
- No more repeats of the same song in charts and mixes.

## [0.3.1] - 2026-10-09

- Lyrics are found for many more tracks: covers and re-uploads (searched by title), titles in Latin and Cyrillic (Monokini = Монокини), titles with "+".
- Genius works again. If it asks you to confirm you're human, a small window with a checkbox shows up and closes by itself.
- No extra heading line in lyrics from Genius.

## [0.3.0] - 2026-10-09

- Likes and follows work again. If SoundCloud wants a captcha, a small window shows up and closes right after. If SoundCloud refuses for now, the like shows in the app at once and reaches the site later, even after a restart.
- Tracks that played on the site but gave 404 in the app now play: SoundCloud's own player plays those, the rest as before.
- Track downloads: right-click a track, "Download". The folder can be chosen, downloaded tracks play offline.
- New "Profile" section (click the avatar): downloaded tracks and likes waiting to be sent.
- Likes made on the site show up in the app by themselves, without the sync button.
- Full-screen mode: volume, repeat, shuffle, the window can be moved.
- "My Wave": the regular wave can be started any time, dislikes are listed and undone one by one.
- Lyrics are found for titles with emoji and tags, karaoke keeps up with long sung words.
- "Go to artist" opens the right artist, no repeated line in Discord.
- A clear message when a track is deleted or blocked in your country.

## [0.2.1] - 2026-10-09

- A desktop shortcut is created on install.
- Settings in one column without frames, the proxy help folds away.
- On the first start ▶ plays your likes from the top.
- Start with the system works reliably: turned on at install, always points to the installed app and is removed completely when turned off.
- Fewer lines: no separators between tracks, above the avatar, at the sliding menu, under the tabs or the window's top bar.

## [0.2.0] - 2026-10-09

- New app icon: the SoundCloud cloud.
- A track plays with one click on its row or cover (▶ on hover).
- Right-click menu: play next, add to queue, like, wave, go to artist, link at the current second.
- New "History" section: tracks, albums and playlists by day.
- The "Playlists" tab now has your own playlists too (marked "Mine").
- "My Wave": "Without liked" doesn't suggest what's already liked or its copies under other names; other versions (speed up, slowed, hardtekk…) stay.
- The left panel is a narrow icon rail that slides out with names on hover.
- Reworked OLED theme: true black and brighter text.
- Switching audio devices (headphones, wireless, speakers) is picked up on the fly, the track keeps playing.
- Fixed the empty likes list; sign in with Google, Facebook and Apple.
- The app keeps no browser running in the tray, updates are checked by themselves.

## [0.1.0] - 2026-10-09

- First version of SC Desk.
- Likes: tracks, playlists, albums and favorite artists.
- "My Wave" with moods and dislikes remembered for good.
- Synced lyrics with word highlighting (LRCLIB, NetEase, Musixmatch, Genius).
- "Now playing" full-screen mode with its own look settings.
- 10-band equalizer, OLED / dark / light themes.
- "Listening to SC Desk" status in Discord.
- Start with the system and updates right in the app.
