# Example carrier board for the ESP32-S3-WROOM-1-N8R8 pack (example-1)

This is a made-up board that goes with the example pack. It is not a product and not a reference design, and its
footprint of the module is simplified. The pack takes the facts below from it as an `open_board` source: they describe
this one board and never override the manufacturer's documents.

Coordinates are millimetres in KiCad's frame (x to the right, y down).

## Board
Board outline: (0, 5.25), (12, 5.25), (12, 12.25), (47, 12.25), (47, 40), (0, 40).
The outline has a notch at the top: there is no base board in x 12 to 47 above y = 12.25, so that the antenna end of the
module hangs over the notch.
Module U1 centre at (36.0, 19.0), not rotated, antenna end towards the top edge.
Mounting hole H1 at (4, 9.25) mm, diameter 3.2 mm.
Mounting hole H2 at (4, 36) mm, diameter 3.2 mm.
Mounting hole H3 at (43, 36) mm, diameter 3.2 mm.

## Free area
Free prototyping area: (7, 16) to (21, 34) mm, on both layers, nothing placed in it. Module pins IO4 and IO5 (pins 4 and 5) are not connected on this board.
Example block: a status LED D1 with a series resistor R2 of 330 ohm in series with D1.
