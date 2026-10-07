// A calibration sheet: where this layout believes the labels are.
//
// Print it on plain paper, hold it against a sheet of the stock, and read off how far the
// printer's page origin sits from the manufacturer's. Correcting that by overriding
// margin_left_mm and margin_top_mm keeps the stock's real geometry recorded in the preset
// rather than bent around one printer.

#let data = json("codes.json")
#let L = data.layout

#set page(paper: "a4", margin: 0pt)
#set text(font: "DejaVu Sans Mono", size: 6pt, fill: black)

#let hair = (paint: black, thickness: 0.2pt)

// Grey, because stock with no side margin puts the first column of labels underneath the
// rulers and a black tick there reads as part of the label.
#let tick = (paint: luma(55%), thickness: 0.2pt)

#let crosshair(x, y) = {
  place(dx: (x - 2) * 1mm, dy: y * 1mm, line(length: 4mm, stroke: hair))
  place(dx: x * 1mm, dy: (y - 2) * 1mm, line(
    angle: 90deg,
    length: 4mm,
    stroke: hair,
  ))
}

#let outline(x, y, n) = {
  place(
    dx: x * 1mm,
    dy: y * 1mm,
    rect(
      width: L.label_w_mm * 1mm,
      height: L.label_h_mm * 1mm,
      stroke: hair,
      inset: 1mm,
      align(center + horizon, text(size: 7pt)[#n]),
    ),
  )
  crosshair(x, y)
  crosshair(x + L.label_w_mm, y + L.label_h_mm)
}

// Rulers along two edges, so a drift can be read off without a second sheet.
#for t in range(0, 21) {
  let x = t * 10
  place(dx: x * 1mm, dy: 0mm, line(angle: 90deg, length: 4mm, stroke: tick))
  place(dx: (x + 0.6) * 1mm, dy: 4.2mm, text(size: 5pt, fill: luma(55%))[#x])
}
#for t in range(0, 30) {
  let y = t * 10
  place(dx: 0mm, dy: y * 1mm, line(length: 4mm, stroke: tick))
  place(dx: 4.2mm, dy: (y + 0.4) * 1mm, text(size: 5pt, fill: luma(55%))[#y])
}

#for i in range(L.cols * L.rows) {
  outline(
    L.margin_left_mm + calc.rem(i, L.cols) * L.pitch_x_mm,
    L.margin_top_mm + calc.quo(i, L.cols) * L.pitch_y_mm,
    i + 1,
  )
}

// At the very foot, where no stock reaches and the vertical ruler has run out.
#place(
  dx: 14mm,
  dy: 292.5mm,
  text(size: 5pt)[
    #data.preset · #L.cols × #L.rows · label #L.label_w_mm × #L.label_h_mm mm ·
    origin #L.margin_left_mm / #L.margin_top_mm mm ·
    pitch #L.pitch_x_mm / #L.pitch_y_mm mm
  ],
)
