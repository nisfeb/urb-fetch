# urb:// in Ladybird

`urb-scheme.patch` teaches [Ladybird](https://github.com/LadybirdBrowser/ladybird) to open `urb://` addresses through urb-fetch. It is made against Ladybird `1f4a6882bf` (2026-10-27).

What it changes:

- `Libraries/LibURL/Urb.{h,cpp}` (new): `urb://~ship/path` becomes `http://ship.urb.localhost:8475/path`, and back. Set `URB_FETCH_PORT` if the gateway runs on another port.
- `Navigable::navigate`: a navigation to `urb://` goes to the gateway, whether it comes from a link, a script, a form or the address bar.
- `LibWebView/URL.cpp`: an `urb://` address typed in the address bar opens through the gateway, and the address bar shows a gateway page as its `urb://` address.
- `Tests/LibURL/TestURL.cpp`: a test of both directions.

`compositor-vulkan-include.patch` is not part of it. It is a fix to Ladybird's build: the Compositor executable includes LibGfx's Vulkan header without linking LibGfx, so the build fails where the system has no `vulkan-headers` package. It may not be needed on later Ladybird commits.

## Use

```
cd ladybird
git checkout 1f4a6882bf
git apply ../urb-fetch/contrib/ladybird/urb-scheme.patch
./Meta/ladybird.py build
```

Then run urb-fetch and open `urb://~sampel-palnet/` in Ladybird.

## Tested

On Linux, against live ships on 2026-10-27:

- `TestURL`: all 43 cases, the new one included.
- An `urb://` address given on the command line (the address bar's path): Ladybird asked the gateway and rendered the page.
- A page whose script sets `location.href` to an `urb://` address: Ladybird asked the gateway for it. Headless text mode doesn't dump a page reached by a script, with or without `urb://`, so the page itself was not read back there.
- Not yet tried in the windowed browser, so the address bar's display of the `urb://` address is covered by the unit test only.

At that commit, Ladybird's compositor exited at startup on the test machine ("open: Permission denied", then "Compositor process disconnected while creating presentation connection") unless run with `--disable-sandbox`. That happens without the patch too.
