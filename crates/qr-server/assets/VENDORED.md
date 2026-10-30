# Vendored assets

## `vendor/alpine.js`

Alpine's CSP build, `@alpinejs/csp@3.15.0`, from
`https://cdn.jsdelivr.net/npm/@alpinejs/csp@3.15.0/dist/cdn.min.js`.

```
sha256  e7c863da5896692f5d478e10dae089e6164fed1c5c1b6a1f7cbbf7a9b093f055
```

The CSP build rather than the ordinary one: Alpine normally evaluates attribute
expressions with `new Function`, which needs `'unsafe-eval'` in the Content-Security-
Policy. This service embeds untrusted text — captions, forwarding destinations — into
the page, so the one mitigation that matters is the one that is not disabled.

The cost is that attributes take a method or property name rather than an expression,
so `x-on:click="remove"` works and `x-on:click="count++"` does not. Everything lives in
the `Alpine.data` components in `app.js`.

Updating it means changing the version and the checksum here, in that order, and
checking that no component started relying on an inline expression.

## `vendor/jsqr.js`

jsQR 1.4.0, from `https://cdn.jsdelivr.net/npm/jsqr@1.4.0/dist/jsQR.js`.

```
sha256  bc40c8a15196236b2314db0856f72ca0b49980cd5413b8c852a7349f5fee0859
```

License: Apache-2.0.

The single-file UMD build. It exposes `window.jsQR`, which is what the link card's
camera loop calls on frames from the video element. The source is vendored rather than
loaded from a CDN so the decoder is part of the same immutable, content-versioned asset
set as the rest of the shell: a deploy cannot pair new markup with an old decoder.

Updating it means replacing the file, changing the version, source and checksum here,
and re-running the link card's camera test.
