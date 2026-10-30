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
