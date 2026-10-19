# Changes

English twin of `CHANGELOG.md`: same versions, same headings `## [x.y.z] - date`.
The release notes carry both; the "Update available" window shows the app's language.

## [1.4.8] - 2026-10-20

- Equalizer: the graph above the bands no longer breaks the layout (the bands are across the whole width again, the labels do not run together).
- Phone: likes, follows and playlists (add a song, remove, create, delete) are now tried by the phone itself, without the computer. If SoundCloud refuses, it still asks the connected computer to do it.
- Phone: a long press on any song in any list and inside a playlist opens a menu (play next, to the end of the queue, to a playlist, to a shared playlist, a link, open on SoundCloud, download, remove from likes). Your own playlist has "Remove from this playlist", and "Likes, Playlists" has "New playlist".
- Phone: screens (settings and others) no longer shrink when a spot that is not a button is pressed.
- Phone: the English language should no longer crash the app (the translation is protected from a failure on long texts).

## [1.4.6] - 2026-10-19

- Rooms: on entering, the program waits for the host for 7 seconds instead of 2.5. With a slow connection a newcomer used to miss the host, declare itself a second host of an "empty" room with the old saved queue, and the two stopped seeing each other. Cases with two hosts and unreadable messages are written to the log, to find the cause.
- Equalizer: a frequency-response graph above the bands, a "Reset" button, a double click on a band puts it back to zero, the wheel over a band changes it by 0.5 dB.
- Phone: the "Version available" window now scrolls, the "Later" and "Update" buttons are always in sight, and the window is shown over the settings if you pressed the update check there. The text is in the app's language.

## [1.4.5] - 2026-10-19

- Corners of the window: "Sharp" in settings now takes the rounding away from the program's window itself too (it used to change only the buttons and cards).
- An anonymous count of people: once a day the program sends the developer a random installation number, the version and the system, to know how many people use it. No name, no account, no address, nothing about what you listen to. It can be switched off in settings: on the computer at the bottom of "System and memory", on the phone at the bottom of "About".
- Phone: the language of the app (Russian and English), the switch is in "Appearance".
- Phone, track menu: "Play next", "Add to the end of the queue", "Add to a playlist" (and a new playlist), "To a shared playlist", "Open on SoundCloud", "Share a link", "Link at the current second", "Search the lyrics again", playback speed.
- Phone: a long press on your own playlist in "Likes" offers to delete it. An open playlist and an artist have "Shuffle", "Like all" and "Follow".
- Phone, lyrics: a translation of the lines into Russian, "Copy the text", the size of the letters (A− and A+).
- Phone: in "Likes" the queue of likes that SoundCloud did not take is shown, with "Send now".
- Phone: "Appearance" has the roundness of the corners (sharp, usual, round) and own theme colours (background, text, accent).
- Phone: "Error report" in "About" collects the details and the end of the log and sends them the way you pick.
- A table of what the computer has and what the phone has (PARITY.md in the project).

## [1.4.4] - 2026-10-19

- Rooms: the "Online" counter no longer breaks. A member whose status message was lost used to stay "not online" for good although they were in the room and adding tracks. Now everyone repeats their status every few seconds.
- Rooms: with the text panel open, the "Invite" and "Call a friend" buttons no longer sit on top of the title and the counter, they move below them.

## [1.4.3] - 2026-10-19

- The volume by the mouse wheel changes 1 percent a notch, in full screen and on the bottom bar. It used to stick on one point.
- The search for a song's text by the words heard has a second source (NetEase): when Musixmatch refuses, the text is still found.
- Settings, "Phone": a "Get the phone app" button with a link and a QR code to download it with the phone's camera.
- Phone: the computer's queue is shown when the music plays on the computer, and tracks can be switched in it.
- Phone: importing a playlist by a link from another service (the connected computer does it).
- Phone: VPN servers can be added without a computer by pasting a subscription link or one server link.
- Phone: the SoundCloud sign-in window goes through the bypass of the blocking, not directly, so it no longer hangs where the site is closed.
- Phone: updates are checked every hour instead of every 12 hours and are taken from GitHub's release list without the cache delay.

## [1.4.2] - 2026-10-19

- A memory leak of the Windows audio service (audiodg): with an output device picked in Settings, the program listed all the sound devices twice a second, and the service grew by gigabytes in a day. The list is now read rarely.
- Moving the data into the program folder is now safe: the data is copied whole and put in place only when all of it went. If some file is still held by the old copy of the program, it works on the old data this time and the move is tried again at the next start. If the settings or the VPN engines went missing after 1.4.1, they come back by themselves.

## [1.4.1] - 2026-10-19

- The program's data now lives in the `data` folder next to sc-desk.exe: settings, cache, covers, engines and models. Nothing is left in AppData. The first start moves the old data over by itself. The «delete data» box of the uninstaller removes this folder too.
- Tracks start faster: the next two tracks are downloaded ahead instead of one, connections to SoundCloud live longer, a track's pieces are fetched in ten streams.
- Likes stuck in the queue: one dead track no longer holds the others back, and tracks removed from SoundCloud leave the queue by themselves.
- The Alt preview no longer starts a song over the playing one if Alt was let go while the track loaded. Alt with another key (Alt+Tab) does not trigger it.
- A safeguard against two sounds at once: a sound left from the previous track stops by itself.
- The lyrics search finds songs with the title in reverse order, and the words hit the beat more exactly, especially in fast reading.
- Visualizer modes, volume by the mouse wheel in full screen and the equalizer, a movable Ctrl+K, a button to remove a track from a playlist, corner roundness in the theme.
- My Wave is stricter about mood and dislikes and has its own settings.
- Auto-pause no longer reacts to screen recorders.
- Android: «Download all» in likes (on Wi-Fi), word highlighting, downloads, rooms and charts closer to the computer version.

## [1.4.0] - 2026-10-17

- SC Desk for Android (the file "SC-Desk-1.4.0-android.apk" in this release). It pairs with the computer by a QR code: Settings, "Connect a phone". The SoundCloud sign-in goes over from the computer without a password, and if the computer is not signed in, the phone can sign in for it.
- A "Play on…" button in the player bar: music can be moved to the phone and back, the volume is steered and what plays on the other device is seen. The phone and the computer can each play their own track.
- The equalizer is shared between the computer and the phone: what is set on one is applied on the other.
- The servers of the VPN subscription can be given to the phone with one button (the computer asks first). The subscription does not count the phone as one more device.
- The phone shows friends, shared playlists and rooms through the computer and searches songs by words from the lyrics. In a room the phone can listen together with the computer.
- My wave takes how you listen into account: how much of a track you heard, where you skipped, what you replay. There is a row of moods, a pick by what people keep next to a track in playlists, and "hearing": good candidates are listened to in the background first and judged by their sound.
- Stats: genres, a choice of period and earlier days.
- The program tidies the disk after itself: old update installers and an overgrown cache of the built-in browser.

## [1.3.0] - 2026-10-16

- The desktop shortcut no longer comes back after every update: it is made once, at the first install.
- A "Own server" subscription with a hundred servers is saved now (the Windows store used to refuse it because of the size).
- The network mode ("Direct" or a proxy) is remembered as soon as you pick it, no need to press "Apply". If a proxy is on and the VPN program is off, the message says plainly what to do. New buttons: "Network status" and "Reset the network".
- In a room a track that did not load for one member no longer starts over and over: the others listen on.
- The preview (Alt over a track) can no longer be left playing over a song.
- My Wave does not put songs in a script your likes have none of (Indian, Arabic, hieroglyphs and so on).
- "Duplicates" in likes: the same song from different uploaders is found and removed in a couple of clicks.
- Plates over games on a hotkey: "now playing" (Ctrl+Alt+N) and the current line of the lyrics (Ctrl+Alt+T). They take no focus and no clicks, the game notices nothing.
- "For streaming (OBS)": a page with the song that plays and the current lyric line, reachable only on your computer (Settings, System).
- Translation of lyric lines: a button in the lyrics panel, the translation goes under each line, into the language of the app.
- Ctrl+Alt+Y: press when a line of the lyrics begins and the text moves so that it begins right now.
- History: search by title and artist, a day is split into night, morning, afternoon and evening.
- "Quieter at night": in the hours you pick the volume goes down smoothly and comes back in the morning.
- "Accent from the cover": the colour of the app follows the cover of the playing track. "List density": compact, normal or large.
- Long mixes and podcasts (20 minutes and more) continue from where you stopped.
- Smart shuffle never plays two songs of one artist in a row. In Ctrl+K inside a room the chosen track goes to the room's queue.
- An error report in one button (Settings, System): a file on the desktop with the version and the end of the log. Tokens, keys, subscription links and the Windows user name are cut out of it.
- Rooms: the host can hand the room over to a chosen member and move the queue by dragging, a track can be skipped by a vote (the host can turn it off), the playing track shows who added it, and when the host leaves the app says who will take over.

## [1.2.4] - 2026-10-14

- Charts are full again: after the cache clean-up in 1.2.1 most songs showed as "not found on SoundCloud".
- Your own server: "Take from a VPN app" takes the servers Koala (and other Clash apps) already saved on the computer, for services that give the subscription only to their own apps.

## [1.2.3] - 2026-10-14

- Your own server: Koala / Clash subscriptions are read too, with all their servers in the list. Servers with xhttp or the new VLESS encryption are marked unsupported.

## [1.2.2] - 2026-10-14

- Your own track from My files now reaches the others in a room and plays for everybody. It could get stuck on "loading" before.
- Your own track shows its length in a room, and its menu has "To the room".
- The heart of your own track puts it in Likes (a track from a room is saved to My files too), and a sync keeps such likes.

## [1.2.1] - 2026-10-14

- The mini player over a full-screen game or video listens again: rest the pointer on it for a moment and the buttons, volume and seeking work. Clicks used to go through it.
- When the provider breaks the connection, a track resumes from where it stopped instead of starting over. When the built-in bypass can't cope, the app says so and suggests your own server.
- The app takes far less disk space: browser cache up to 8 MB, covers up to 12 MB, the library compacted. The old overgrown cache is cleared by itself. A new Storage section in Settings shows the sizes and has clean-up buttons.
- Your own server: choose what to introduce itself as to the subscription (Happ, v2rayN, Hiddify and others). Stubs like "Install Happ" are recognised, and a dead server is dropped at once instead of cutting SoundCloud off.
- Tracks in likes and your playlists can be dragged: hold a track and move it. A playlist's order is saved on SoundCloud, the likes' order stays in the app.
- A liked track shows in Likes at once, no sync needed.
- When you skip, the list follows the playing track.
- Recognising a song by its words finds more, and for remixes and sped-up edits shows the text without timings so the lines don't run past the voice.

## [1.2.0] - 2026-10-14

- Your own server instead of a VPN app: paste a subscription from Happ, v2rayN and the like (Settings → Network) and pick a server from the list. Only SoundCloud goes through it, the VPN app can be closed. The engine (sing-box) is downloaded once, when you pick a server.
- My files (Profile): your own tracks that aren't on SoundCloud. They get in only through the app, checked to be real sound, packed into Opus with no audible loss (pitch, loudness and length kept) and kept in one encrypted file. Nothing can be slipped in around the app.
- In a room you can play a track from My files: it goes to the others over the room's encrypted channel and plays for everybody. Liked it? Right click → "Save to My files".
- When a track won't play, the app says why and what to do: removed from SoundCloud, still processing after an upload, blocked in your country, Go+ only.
- Recognising a song by its words is more accurate: when Musixmatch fails it searches Genius, beat noise is not taken for words, and songs are not matched by common words.

## [1.1.0] - 2026-10-14

- If your provider blocks SoundCloud, the app gets around it by itself, with no VPN and no third-party servers: the start of the connection is cut so the filter can't see the site name, and SoundCloud's address is asked over secure DNS. Works in the sign-in window too. Settings → Network → "Get around a SoundCloud block". "Auto" turns on only when SoundCloud doesn't open directly, so it doesn't clash with zapret or a VPN.
- Google sign-in: the sign-in window is no longer refused as an "unsafe app".
- Songs are recognised by their words: when no lyrics turn up by the name, the app listens to 15 seconds and finds the song by what it hears. About a second of work, once per track. The model (57 MB) is downloaded the first time it's needed. Settings → Lyrics.
- Lyrics turn up more often: for re-uploads the real artist is found by the title and length. If a lyrics source didn't answer, "not found" is no longer kept for three days.
- Likes without the internet (Downloads → switch): a liked track that plays is saved from the audio already loaded, the rest are downloaded little by little.
- Hold Alt over a track: a bit from the middle plays quietly while the current track goes down. Let go and everything is back.
- Silence at the start and end of tracks is skipped: tighter transitions.
- Seeking without clicks.
- My Wave: language of the songs (only Russian or only foreign) and "Nothing heard before".
- Opus tracks play.
- Go+ stand-ins are found faster, tracks through the second account start faster.
- The mini player is wider, the volume is no longer cut off.

## [1.0.1] - 2026-10-14

- Manual sign-in is simpler: only the oauth_token is needed (it can be taken from the cookie on soundcloud.com), the app finds the client_id itself. When you are not signed in, the manual block is open at once.
- A second account (Settings → SoundCloud account): if the subscription is on another account, paste its oauth_token. The main account with all its likes, playlists and friends stays, and the tracks it cannot play in full play through the second one. Nothing else to do.

## [1.0.0] - 2026-10-14

The first "1.0": everything that piled up, put together and checked.

- Lyrics search (Ctrl+K, "By lyrics"): the liked songs with these words come first, below them the songs of all SoundCloud that have them in their lyrics.
- The "Add a track" window of a shared playlist: the "My likes" tab shows your likes at once.
- Mini player: next to the pause button the heart shows whether the song is liked, and flips it on a click. A like made in the big window reaches the mini player at once.
- Mini player: the pin in the open panel keeps the capsule on top of all windows or lets it go.
- Mini player in games: the capsule lets clicks through while a game is in play and takes them when the game shows the plain Windows pointer (a pause menu, a console). A game's own pointer, like the one in Dota, is not mistaken for it. Ctrl+Alt+K makes the capsule clickable in a game on demand; Settings choose for how many seconds or until the key is pressed again.
- Mini player: the lyric line has its own opacity (without lyrics the title fades with the capsule).
- Lyrics: they show up noticeably sooner. The next song's lyrics are fetched ahead while the current one plays, and the lyric sources are asked at the same time, not one after another.

## [0.9.9] - 2026-10-14

- Fixed: some tracks did not play, without any error. SoundCloud serves such tracks only as HLS AAC (in fragments), and the app could not decode that. The fragments are now unpacked into a plain AAC stream, and the tracks play, seeking included.

## [0.9.8] - 2026-10-14

- Mini player: the next lyric line shows a little earlier (about 0.8 s), so you can see what is coming; the words are still lit by the real time.
- Mini player: the lyric line falls behind less when the window is covered by a full-screen app, and it follows your lyrics shift from the settings.

## [0.9.7] - 2026-10-13

- Fixed: music did not play with the error "deleted, closed by the author or not available in your country". SoundCloud stopped serving some tracks in one of the formats; now, when a format is not served, the next one (AAC) is taken.

## [0.9.6] - 2026-10-14

- Playlists can be deleted: "Delete playlist" in the right-click menu and a "Delete" button on the page of your own playlist (with a confirmation).
- More than 500 songs: the parts "Name (1/3)", "(2/3)"… show as one playlist in "Playlists". It plays as a whole (play, shuffle, one list) and is deleted as a whole. The order and the contents are changed in the separate parts.
- Shared playlists: the banner shows its size (1200 × 300, 4 : 1), and a playlist can have its own picture (or keep the artwork of the first tracks).
- Shared playlists: the artwork of the tracks shows at once, not after the first visit to each playlist; the playlist page opens faster.

## [0.9.5] - 2026-10-13

- Songs marked "Don't recommend" are never played now: in a playlist, the queue, the wave and a room such a song is skipped and the next one plays at once.

## [0.9.4] - 2026-10-13

- "Like all" on a playlist now works at once: the hearts light up on every song as you press it, and the likes go to SoundCloud in one run in the background, without pauses. The songs are liked so that the playlist lies in its own order in "My likes". The run can be stopped.
- The lyrics panel on the right can be widened to the left: drag its left edge (a double click returns the width), and the "A−" and "A+" buttons in its header change the text size.
- The volume is changed with the mouse wheel when the cursor is over the slider or the speaker icon.

## [0.9.3] - 2026-10-13

- Import from Yandex Music and Spotify no longer cuts the list at 500 songs: a long playlist is split into several SoundCloud playlists ("Name (1/3)", "(2/3)"…), because SoundCloud holds up to 500 songs in a playlist.
- If the strict search does not find a song, the import tries a similar title (the same artist, a close length, no covers or remixes).
- A "Like all" button on the playlist page: likes every song of the playlist one by one, and can be stopped.

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
