"""show_tx.py for a screenshot: a pty draws no images, so each picture's cells and PNG go to SHOT_PICTURES, for the SVG to
lay the real images over the half blocks. Its cells are sized like term2svg's, so a chart fits its box exactly."""
import base64, json, os, sys
from collections import namedtuple

sys.path.insert(0, os.getcwd())
import show_tx  # noqa: E402

CellSize = namedtuple("CellSize", "width height")
show_tx.get_cell_size = lambda: CellSize(17, 41)
pictures, shown = {}, show_tx.Picture.show_image


def show_image(self, data, cells):
    shown(self, data, cells)

    def record():
        image = self.query_one(show_tx.Image).region
        pictures[id(self)] = {"x": image.x, "y": image.y, "w": image.width, "h": image.height, "png": base64.b64encode(data).decode()}
        with open(os.environ["SHOT_PICTURES"], "w") as handle:
            json.dump(list(pictures.values()), handle)
    self.app.set_timer(0.3, record)


show_tx.Picture.show_image = show_image
show_tx.ShowApp().run()
