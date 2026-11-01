# iOS arrival motion study

This is a visual prototype only. It does not change the production iOS app.
Design decisions and evidence limits: [design draft](../../docs/specs/ios-arrival-motion.md).

Serve this directory with a static HTTP server and open `index.html` to replay or scrub each scene.
The backgrounds are Chinese mock screenshots of the current iOS app; the navigation accessory and
typing in the video are proposed UI. `mac-motion.js` is generated from the existing
`src/views/popup/inboxArrival.ts`. The bubble geometry is reused from `InboxArrivalNotice.vue`;
its Lucide ISC notice is in `assets/lucide-license.txt`.

`render.mjs` needs Node, FFmpeg, Playwright Core, and a local Chromium executable.
Set `AH_MOTION_PLAYWRIGHT` to the absolute path of Playwright Core's `index.mjs`, and
`AH_MOTION_CHROMIUM` to Chromium. Run `node demo/ios-arrival/render.mjs --preview` for
keyframes and prototype input checks, or omit `--preview` for a 60 fps H.264 video.
Outputs are in the ignored `output/` directory. The default video contains the four A scenes;
`--compare` includes the two B scenes from the first review.

The user chose A and, after comparing an alternative upward arc, chose to keep the original Mac
trajectory. The active prototype uses `arrivalFrame` for both destinations.
