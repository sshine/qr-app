# /kiksmiks label pages, 2026-11-01

Screenshots from the real server and the real browser, against a throwaway
database seeded with six used labels: three photo albums, two forwards and one
link. The photos are generated gradient test images, not personal photographs.

| File | Viewport | What it shows |
| --- | --- | --- |
| `gallery-phone.png` | 390×844 | The gallery as a list. Only labels that have been used are present; the unused stickers from the same print run are absent. Title and thumbnail are links to the label page. |
| `gallery-desktop.png` | 1440×900 | The gallery as label-shaped cards, three across at this width. The card width was raised from 20rem to 23rem because a title, code, place, coordinates, tags and the delete control overflowed the fixed-height card and clipped the last line. |
| `label-phone.png` | 390×844 | One label's page: photos, editable title, tags with add/remove, place with existing/new/clear, release, delete, and the Open link. The hint says opening the label needs the sticker's secret, which the overview never holds. |
| `label-desktop.png` | 1440×900 | The same page at desktop width. The tag and place controls stay in one reading column rather than stretching. |
| `label-released-phone.png` | 390×844 | A released forward label: no Open link, a "released" marker, and the "Previous uses" list with Recover. |

Checked against the rest of the app: the action row's buttons and the green Open
link have the same 45px height; form inputs and selects match the existing
radius and padding; the disabled "Tag this new place" button is visibly faded
until the device supplies coordinates; no horizontal scrolling at either width;
the desktop gallery no longer clips the last line of a tall card.
