# Sky Glance — Mira weather dashboard

`SkyGlance.html` is the installable app (Mira: **Apps → Create an app → Upload → Check code → Add → Install → Open**).

**Glasses** (swipe to change tabs; the dots top-right show where you are):

| Tab | Shows | Tap |
| --- | --- | --- |
| Now | Big temperature, condition icon, high/low, feels-like, humidity, wind; next 12 h of rain-chance and sunshine bars | Refresh |
| Hourly | 24 h table: temp, rain %, inches, sunshine %, UV, wind; today's sunrise/sunset, dew point, rain total | Toggle hours 1–12 / 13–24 |
| Wind | Current wind compass and gusts; 12 h wind bars with gust ticks and heading arrows | Refresh |
| Rain & wind map | 7×7 grid over a 25-mile radius, shaded by rain intensity with wind arrows, animating through 12 hours | Pause / play |

**Phone companion:** gets location (GPS → saved city/ZIP → last GPS fix), fetches
[Open-Meteo](https://open-meteo.com) (no API key), refreshes every 15 minutes,
and shows the same data plus a scrubbable map. Imperial units.

## Build

```sh
rustup target add wasm32-unknown-unknown
python3 WeatherApp/package.py SkyGlance.html
python3 CompanionExample/validate.py SkyGlance.html
node test/smoke.js SkyGlance.html test-out   # headless end-to-end check (Playwright)
```

Sources: `WeatherApp/weather.rs` (glasses), `WeatherApp/companion.html` (phone).
`Shared/` and `CompanionExample/` are the unmodified Mira SDK files.
