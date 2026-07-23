import json
import pathlib
import unittest


ROOT = pathlib.Path(__file__).parents[1]
WEB = ROOT / "web"


class PwaTest(unittest.TestCase):
    def test_manifest_and_icons_are_linked(self):
        index = (WEB / "index.html").read_text()
        manifest = json.loads((WEB / "manifest.webmanifest").read_text())

        self.assertIn('rel="manifest" href="manifest.webmanifest"', index)
        self.assertEqual(manifest["start_url"], "./")
        self.assertEqual(manifest["display"], "standalone")
        self.assertEqual(manifest["theme_color"], "#0e1116")
        self.assertEqual(manifest["background_color"], "#0e1116")
        self.assertEqual(
            {(icon["sizes"], icon["purpose"]) for icon in manifest["icons"]},
            {
                ("192x192", "any"),
                ("512x512", "any"),
                ("512x512", "maskable"),
            },
        )

        for icon in manifest["icons"]:
            self.assertTrue((WEB / icon["src"]).is_file())

    def test_service_worker_caches_runtime_and_is_registered(self):
        index = (WEB / "index.html").read_text()
        main = (WEB / "main.js").read_text()
        service_worker = (WEB / "service-worker.js").read_text()

        self.assertIn('rel="apple-touch-icon"', index)
        self.assertIn("serviceWorker.register('service-worker.js')", main)

        for asset in (
            "./",
            "./index.html",
            "./main.js",
            "./worklet.js",
            "./tunerfish.wasm",
            "./manifest.webmanifest",
            "./icons/icon-192.png",
            "./icons/icon-512.png",
            "./icons/icon-maskable-512.png",
        ):
            self.assertIn(f"'{asset}'", service_worker)


if __name__ == "__main__":
    unittest.main()
