"""Pick and download one Curiosity Navcam image per stop (approach view), by a FIXED rule (no hand-picking):
the earliest non-thumbnail, full-frame (EDR_F) NAV_LEFT_B image taken at the stop's PLACES site and drive on the stop's sol.
Source: NASA raw-images feed https://mars.nasa.gov/api/v1/raw_image_items/ (undocumented API; checked).
Credit: each item's `image_credit` (Navcam: "NASA/JPL-Caltech"); JPL image use policy https://www.jpl.nasa.gov/jpl-image-use-policy .
Writes assets/photos/<imageid>.jpg and fills the `photo` field of tools/sources/stops_source.json. Stops with no matching image keep photo = null.
Usage: python tools/fetch_photos.py
"""
import json
import os
import urllib.request

import config
SRC = config.STOPS_SOURCE
API = ("https://mars.nasa.gov/api/v1/raw_image_items/?order=sol%20desc&per_page=200&page=0&condition_1=msl:mission"
       "&condition_2={sol}:sol:gte&condition_3={sol}:sol:lte&search=NAV_LEFT_B")


def pick(stop):
    items = json.load(urllib.request.urlopen(API.format(sol=stop["sol"]), timeout=60))["items"]
    ok = [i for i in items if str(i.get("site")) == stop["site"] and str(i.get("drive")) == stop["drive"]
          and not i.get("is_thumbnail") and "EDR_F" in i.get("imageid", "") and i.get("instrument") == "NAV_LEFT_B"]
    ok.sort(key=lambda i: i.get("date_taken") or "")
    return ok[0] if ok else None


def main():
    src = json.load(open(SRC, encoding="utf-8"))
    os.makedirs(os.path.join(config.ASSETS, "photos"), exist_ok=True)
    for s in src["stops"]:
        it = pick(s)
        if it is None:
            print(f"{s['id']}: no image matches the rule; photo stays null")
            s["photo"] = None
            continue
        fn = f"photos/{it['imageid']}.jpg"
        urllib.request.urlretrieve(it["https_url"], os.path.join(config.ASSETS, fn))
        s["photo"] = dict(file=fn, imageid=it["imageid"], instrument=it["instrument"], sol=it["sol"], date_taken=it.get("date_taken"),
                          site=it.get("site"), drive=it.get("drive"), credit=it.get("image_credit"), url=it["https_url"],
                          rule="earliest full-frame NAV_LEFT_B at the stop's site/drive on its sol")
        print(f"{s['id']}: {it['imageid']} ({it.get('date_taken')}, credit {it.get('image_credit')})")
    json.dump(src, open(SRC, "w", encoding="utf-8"), indent=1, ensure_ascii=False)


if __name__ == "__main__":
    main()
