// The client. Everything here is an enhancement: every control it touches is a form
// that already works without it.
//
// Alpine's CSP build is what is loaded, so an attribute names a method or a property
// and never an expression. That is why these components look like ordinary objects.

(() => {
  "use strict";

  /** The payload the server already rendered the page from, so nothing is fetched twice. */
  function hydrate() {
    const el = document.getElementById("qr-state");
    if (!el) return {};
    try {
      return JSON.parse(el.textContent) ?? {};
    } catch {
      return {};
    }
  }

  const boot = hydrate();

  /** Remembers codes this browser has opened, for the landing page. */
  function remember(code) {
    if (!code) return;
    try {
      const seen = JSON.parse(localStorage.getItem("qr.recent") ?? "[]");
      const next = [code, ...seen.filter((c) => c !== code)].slice(0, 12);
      localStorage.setItem("qr.recent", JSON.stringify(next));
    } catch {
      // A private window, or storage turned off. Nothing here depends on it.
    }
  }

  // The fragment carries the capability and is never sent to a server. Exchanging it
  // for a cookie here is what keeps the secret out of access logs and Referer headers;
  // replaceState then takes it out of the address bar and the history entry.
  async function redeem(code) {
    const secret = location.hash.slice(1);
    if (!secret) return false;

    const response = await fetch(`/c/${code}/api/v1/redeem`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ secret }),
    });

    history.replaceState(null, "", location.pathname + location.search);
    if (!response.ok) return false;
    remember(code);
    return true;
  }

  /** The code and secret in a sticker's own URL, or null for any other QR. */
  function stickerTarget(text) {
    let url;
    try {
      url = new URL(text, location.origin);
    } catch {
      return null;
    }

    // A sticker encodes the whole redeem URL, and the path is uppercase because that is
    // what keeps the symbol in QR alphanumeric mode. A browser may still hand it to us
    // in either spelling after normalisation.
    const match = /^\/[cC]\/([^/]+)$/.exec(url.pathname);
    if (!match) return null;

    const code = match[1].toUpperCase().replace(/[\s-]/g, "");
    const secret = url.hash.slice(1).toUpperCase();
    // The same bounds the server enforces. A stray poster or a website QR should not
    // turn into a claim the server then has to refuse.
    if (!/^[0-9A-Z]{6,12}$/.test(code)) return null;
    if (!/^[0-9A-Z]{12,24}$/.test(secret)) return null;
    return { code, secret };
  }

  /** The four-character groups the code is printed in. */
  function groupCode(code) {
    return code.replace(/(.{4})(?=.)/g, "$1 ");
  }

  /** Says what the visitor can do about each camera failure. */
  function cameraMessage(error) {
    switch (error && error.name) {
      case "NotAllowedError":
      case "SecurityError":
        return "Camera permission was refused. Allow this page to use the camera in the browser's settings, then try again.";
      case "NotFoundError":
      case "OverconstrainedError":
        return "No camera was found on this device.";
      case "NotReadableError":
        return "The camera is being used by another app. Close it and try again.";
      case "NotSupportedError":
        return "This browser cannot open a camera. Scanning needs a browser with camera support.";
      default: {
        const detail = String((error && error.message) || "").trim();
        return detail
          ? `The camera could not be started. ${detail}`
          : "The camera could not be started. Try again.";
      }
    }
  }

  document.addEventListener("alpine:init", () => {
    // The page served before a cookie exists. Its only job is the exchange above.
    // The landing page's list, read from this browser's own storage. The service has no
    // idea who is asking, and a list of every code would be the inventory the whole
    // capability scheme exists to withhold.
    Alpine.data("recent", () => ({
      codes: [],

      init() {
        try {
          const seen =
            JSON.parse(localStorage.getItem("qr.recent") ?? "[]") ?? [];
          this.codes = seen.map((code) => ({ code, href: `/c/${code}` }));
        } catch {
          // Storage off, or cleared. An empty list is the right answer.
        }
      },
    }));

    Alpine.data("bootstrap", () => ({
      async init() {
        document.querySelectorAll(".only-with-script").forEach((el) => {
          el.style.display = "block";
        });
        if (await redeem(boot.redeem)) location.reload();
      },
    }));

    // The bar every issued code carries. Release lives behind a dialog because it is
    // the one control that changes what a scan does; the dialog says that the photos
    // and history are kept before the form is submitted.
    Alpine.data("codeControls", () => ({
      releaseOpen: false,

      openRelease() {
        this.releaseOpen = true;
      },

      closeRelease() {
        this.releaseOpen = false;
      },
    }));

    // The link card's camera. The decoder is jsQR over frames from the video element;
    // the only thing this component puts in the form is the pair it has actually read,
    // and only after the visitor confirms it.
    Alpine.data("linkScan", () => {
      // A MediaStream and a canvas are not reactive state: Alpine would proxy them and
      // every brand-checked method on the stream would then see the proxy.
      let stream = null;
      let canvas = null;
      let context = null;
      let frame = 0;
      let lastScan = 0;

      function stop() {
        if (frame) cancelAnimationFrame(frame);
        frame = 0;
        if (stream) {
          for (const track of stream.getTracks()) track.stop();
          stream = null;
        }
      }

      return {
        showStart: true,
        showCamera: false,
        showResult: false,
        starting: false,
        hasError: false,
        error: "",
        code: "",
        secret: "",
        codeDisplay: "",

        async start() {
          if (this.starting || this.showResult) return;
          this.hasError = false;
          this.error = "";
          this.starting = true;

          try {
            if (
              !navigator.mediaDevices ||
              !navigator.mediaDevices.getUserMedia
            ) {
              throw Object.assign(new Error("no camera support"), {
                name: "NotSupportedError",
              });
            }
            if (typeof window.jsQR !== "function") {
              throw new Error("the QR decoder did not load");
            }

            stream = await navigator.mediaDevices.getUserMedia({
              video: {
                facingMode: { ideal: "environment" },
                width: { ideal: 1280 },
                height: { ideal: 720 },
              },
              audio: false,
            });

            const video = this.$refs.video;
            video.srcObject = stream;
            await video.play();

            this.showStart = false;
            this.showCamera = true;
            lastScan = 0;
            frame = requestAnimationFrame(() => this.tick());
          } catch (e) {
            stop();
            this.showCamera = false;
            this.showStart = true;
            this.hasError = true;
            this.error = cameraMessage(e);
          } finally {
            this.starting = false;
          }
        },

        /** Decodes at most ten frames a second; the camera runs faster than that. */
        tick() {
          if (!this.showCamera) return;
          frame = requestAnimationFrame(() => this.tick());

          const now = performance.now();
          if (now - lastScan < 100) return;
          lastScan = now;

          const video = this.$refs.video;
          if (!video || video.readyState < 2 || !video.videoWidth) return;

          // Scaled down before decoding: a 1280-wide frame is more pixels than a QR
          // needs and ten scans a second of it is not free on a phone.
          const scale = Math.min(1, 640 / video.videoWidth);
          const width = Math.round(video.videoWidth * scale);
          const height = Math.round(video.videoHeight * scale);
          if (!canvas) {
            canvas = document.createElement("canvas");
            context = canvas.getContext("2d", { willReadFrequently: true });
          }
          if (canvas.width !== width || canvas.height !== height) {
            canvas.width = width;
            canvas.height = height;
          }
          if (!context) return;

          context.drawImage(video, 0, 0, width, height);
          const pixels = context.getImageData(0, 0, width, height);
          const result = window.jsQR(pixels.data, pixels.width, pixels.height, {
            inversionAttempts: "dontInvert",
          });
          if (!result) return;

          const found = stickerTarget(result.data);
          if (!found) return;

          stop();
          video.srcObject = null;
          this.code = found.code;
          this.secret = found.secret;
          this.codeDisplay = groupCode(found.code);
          this.showCamera = false;
          this.showStart = false;
          this.showResult = true;
        },

        reset() {
          stop();
          const video = this.$refs.video;
          if (video) video.srcObject = null;
          this.showStart = true;
          this.showCamera = false;
          this.showResult = false;
          this.starting = false;
          this.hasError = false;
          this.error = "";
          this.code = "";
          this.secret = "";
          this.codeDisplay = "";
        },
      };
    });

    // A forward code shows where it goes for five seconds, then leaves. Staying cancels
    // the timer for good; without script the destination and Continue link are all that
    // is needed, and the countdown block stays hidden by x-cloak.
    Alpine.data("interstitial", () => ({
      seconds: 5,
      message: "",
      timer: null,

      init() {
        this.message = this.remaining();
        this.timer = setInterval(() => this.tick(), 1000);
      },

      remaining() {
        return this.seconds === 1
          ? "Continuing in 1 second."
          : `Continuing in ${this.seconds} seconds.`;
      },

      tick() {
        this.seconds -= 1;
        if (this.seconds <= 0) {
          clearInterval(this.timer);
          if (boot.state?.url) location.assign(boot.state.url);
          return;
        }
        this.message = this.remaining();
      },

      stay() {
        clearInterval(this.timer);
        this.message = "Not redirecting. The destination is above.";
      },
    }));

    // The device's own position, for tagging a box where it stands. The form still
    // works without this: picking an existing place needs no coordinates at all.
    Alpine.data("locationTag", () => ({
      lat: "",
      lon: "",
      status: "",
      noCoords: true,

      locate() {
        if (!navigator.geolocation) {
          this.status = "This browser has no location support.";
          return;
        }

        this.status = "Asking this device…";
        navigator.geolocation.getCurrentPosition(
          (position) => {
            this.lat = position.coords.latitude.toFixed(6);
            this.lon = position.coords.longitude.toFixed(6);
            this.noCoords = false;
            this.status = `Got it: ${this.lat}, ${this.lon}.`;
          },
          (error) => {
            this.status =
              error.code === error.PERMISSION_DENIED
                ? "Location permission was refused."
                : "The device did not give a position.";
          },
          { enableHighAccuracy: true, timeout: 10000, maximumAge: 60000 },
        );
      },
    }));

    Alpine.data("album", () => ({
      photos: boot.state?.photos ?? [],
      pending: [],
      undo: null,
      error: "",

      init() {
        remember(boot.code);
      },

      /** Submitted as soon as the camera app returns, so there is no second tap. */
      maybeSubmit(event) {
        const input = event.target;
        if (!input.files || input.files.length === 0) return;
        this.upload([...input.files]);
        input.value = "";
      },

      async upload(files) {
        for (const file of files) {
          // Drawn immediately, greyed, so the photo is on screen before the bytes
          // have finished going the other way.
          const tile = {
            id: `pending-${Math.random()}`,
            url: URL.createObjectURL(file),
          };
          this.pending.unshift(tile);

          const body = new FormData();
          body.append("photo", file);

          try {
            const response = await fetch(
              `/c/${boot.code}/api/v1/h/photo-album/photos`,
              { method: "POST", headers: { accept: "application/json" }, body },
            );
            if (!response.ok) throw new Error(await response.text());
            const added = await response.json();
            this.photos = [...added.photos, ...this.photos];
            this.error = "";
          } catch (e) {
            this.error = String(e.message ?? e).trim();
          } finally {
            this.pending = this.pending.filter((p) => p.id !== tile.id);
            URL.revokeObjectURL(tile.url);
          }
        }
      },

      /** No dialog: the undo below is the confirmation, and it costs one fewer tap. */
      async remove(event) {
        const id = event.currentTarget.dataset.photo;
        const removed = this.photos.find((p) => p.id === id);
        this.photos = this.photos.filter((p) => p.id !== id);

        const response = await fetch(
          `/c/${boot.code}/api/v1/h/photo-album/photos/${id}`,
          { method: "DELETE" },
        );
        if (!response.ok) {
          this.photos = [removed, ...this.photos];
          return;
        }

        this.undo = removed;
        clearTimeout(this._undoTimer);
        this._undoTimer = setTimeout(() => {
          this.undo = null;
        }, 6000);
      },

      async restore() {
        const photo = this.undo;
        this.undo = null;
        if (!photo) return;
        const response = await fetch(
          `/c/${boot.code}/api/v1/h/photo-album/photos/${photo.id}/restore`,
          { method: "POST" },
        );
        if (response.ok) this.photos = [photo, ...this.photos];
      },
    }));

    Alpine.data("forward", () => ({
      url: boot.state?.url ?? "",
      note: boot.state?.note ?? "",
      saving: false,
      error: "",

      async save() {
        this.saving = true;
        this.error = "";
        try {
          // The handler's own mount, taken from the payload rather than written out,
          // so this component does not have to know what it is called.
          const response = await fetch(
            `/c/${boot.code}/api/v1/h/${boot.handler}/target`,
            {
              method: "PUT",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({ url: this.url, note: this.note || null }),
            },
          );
          if (!response.ok) throw new Error(await response.text());
        } catch (e) {
          this.error = String(e.message ?? e).trim();
        } finally {
          this.saving = false;
        }
      },
    }));
  });

  // Registered at the root, because the scope has to cover /c/, and a worker scoped to
  // /assets/ would see none of the pages it exists to make load instantly.
  if ("serviceWorker" in navigator) {
    addEventListener("load", () => {
      navigator.serviceWorker.register("/sw.js").catch(() => {
        // An unsupported browser, or a private window. The site works without it.
      });
    });
  }
})();
