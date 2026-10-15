# Privacy Policy

_Last updated: October 15, 2026_

Windgets is a desktop app that runs entirely on your computer. It has no
accounts, no telemetry, no analytics and no server of its own. The author
receives no data from the app at all.

## What stays on your computer

Settings and widget data are stored locally:

- `%APPDATA%\io.github.commonfool.windgets` — settings, notes, to-do lists,
  mail accounts, 2FA accounts, speedtest history;
- `%LOCALAPPDATA%\io.github.commonfool.windgets` — downloaded tools and the
  error log.

Mail passwords and 2FA secrets are encrypted with Windows DPAPI for the
current Windows user. Clipboard history is kept in memory only and is never
written to disk. Uninstalling with "Delete app data" removes both folders.

## What leaves your computer, and when

The app contacts a service only when the widget that needs it is enabled, and
sends only what that service needs to answer:

| Widget | Service | What is sent |
|---|---|---|
| Weather | [Open-Meteo](https://open-meteo.com/en/terms) | The city you search for and its coordinates |
| Speedtest | [Measurement Lab](https://www.measurementlab.net/privacy/) | Test traffic, only when you press the button. M-Lab publishes test results, including your IP address, as an open dataset |
| Network | Cloudflare `1.1.1.1`, Google `8.8.8.8`, OpenDNS `208.67.222.222` | ICMP ping packets |
| Spotify | [Spotify](https://www.spotify.com/legal/privacy-policy/) | Your sign-in (via Spotify's own page) and player commands, using your own Client ID |
| Mail | Your mail server | Your login and password, over IMAP with TLS |
| Updates, converter, downloader, CPU driver | [GitHub](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement) | Requests for release information and downloads |
| Downloader | The site of the link you paste | Whatever yt-dlp needs to fetch that link |

Update checks run once a week and can be turned off in Settings → General.
Tools (ffmpeg, yt-dlp, QuickJS, the PawnIO driver) are downloaded only after
you agree.

Each service handles your data under its own privacy policy, linked above.

## Your rights (GDPR)

If you are in the European Union or the European Economic Area:

- **The author is not a controller of your data.** Windgets has no server and
  the author receives nothing, so there is nothing the author could show,
  correct or delete for you.
- **Data on your computer is yours.** You can view, change or delete it at any
  time: it lives in the two folders listed above, and uninstalling with
  "Delete app data" removes both. Disconnecting Spotify in the widget settings
  removes the sign-in and the last played track.
- **Speedtest.** Pressing the speedtest button is your consent to the test;
  the widget settings link to M-Lab's privacy policy and rules. Measurement
  Lab is the controller of the published results. To exercise your rights over
  them, contact M-Lab as described in its
  [privacy policy](https://www.measurementlab.net/privacy/). Results already
  published as open data may not be removable.
- **Other services** (Open-Meteo, Spotify, GitHub, your mail provider) are
  controllers of what they receive; contact them under their own policies,
  linked above.
- You also have the right to lodge a complaint with your local data
  protection authority.

## Contact

Questions about this policy: open an issue at
<https://github.com/Common-Fool/WindGets/issues>.
