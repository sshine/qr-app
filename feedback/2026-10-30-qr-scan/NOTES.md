# Camera scanning when linking a QR code

All shots are the real chooser at `http://127.0.0.1:8081` from the throwaway e2e
server, with a clean PNG rendered by the app's own encoder (`qrcode` 0.14.1) fed to
`getUserMedia` as a canvas capture stream. Phone is 390×844, desktop is 1440×900.

| File | State |
| --- | --- |
| `chooser-phone.png`, `chooser-desktop.png` | The link card after the typed fields were removed. One full-width "Scan the other sticker" button; no text inputs. |
| `scanning-phone.png`, `scanning-desktop.png` | Live camera preview inside the card with the guide frame, the "Point the camera…" status and a "Stop camera" control. The decoder was paused only for the screenshot, so the frame stays on screen. |
| `found-phone.png`, `found-desktop.png` | After jsQR decoded the sticker: the grouped target code and the "Use this sticker" / "Scan another" confirmation. The claim is not sent until "Use this sticker" is pressed. |
| `permission-error-phone.png`, `permission-error-desktop.png` | `getUserMedia` rejected with `NotAllowedError`: the card explains how to allow the camera and leaves the code unclaimed. |

Checked in the images: the card keeps the same radius, border and spacing as the
other chooser cards; the icon is still constrained to its column; title and blurb
stay stacked; the scan and confirmation buttons are the same height and the primary
one is accent-coloured; the preview is full width on a phone and does not introduce
horizontal scroll; the error wraps as prose rather than running under the button.
The e2e suite additionally claims through the decoded pair and asserts the redirect
landed on the other sticker's album.
