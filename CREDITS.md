# Credits and sources

VISOR is an independent NASA Space Apps Challenge 2026 entry. **It is not endorsed by NASA, JPL, Caltech, the USGS, the
University of Arizona or any mission team.** No NASA or JPL logos are used. Every data set and source used is listed below
(Space Apps 2026 terms: "You must cite every source that is used in the creation of your project").

## Data

| Data | Credit line | Where | Terms |
| --- | --- | --- | --- |
| MSL Gale Merged DEM 1 m (v3) | Calef III, F. J., and Parker, T. (2016), MSL Gale Merged DEM 1m, PDS Annex, U.S. Geological Survey | [Astropedia record](https://astrogeology.usgs.gov/search/map/mars_msl_gale_merged_dem_1m); [file](https://planetarymaps.usgs.gov/mosaic/Mars/MSL/MSL_Gale_DEM_Mosaic_1m_v3.tif) | "Please credit authors"; Use Constraints: None |
| MSL Gale HiRISE colour orthophoto mosaic, 25 cm | Parker, T., and Calef III, F. J. (2016; colour version July 2025), MSL Gale Merged Orthophoto Mosaic, PDS Annex, U.S. Geological Survey. HiRISE images: NASA/JPL/University of Arizona | [Astropedia record](https://astrogeology.usgs.gov/search/map/mars_msl_gale_merged_orthophoto_mosaic_25cm); [COG](https://asc-pds-services.s3.us-west-2.amazonaws.com/mosaic/Mars/MSL/MSL_Gale_HiRISE-LRGB_78quads_sharp_cog.tif) | "Please credit authors"; HiRISE images are public domain, credit requested ([HiRISE usage](https://uahirise.org/media/usage.php)) |
| HiRISE DTM DTEEC_009149_1750_009294_1750_U01 (data readiness check only; not shipped) | NASA/JPL/University of Arizona (HiRISE stereo pair PSP_009149_1750 / PSP_009294_1750) | [DTM page](https://www.uahirise.org/dtm/dtm.php?ID=PSP_009149_1750) | HiRISE products are public domain, credit requested ([HiRISE usage](https://uahirise.org/media/usage.php)) |
| Astronaut 3D model (Walk mode, third person) | NASA 3D Resources, "Astronaut" (Astronaut.glb); split into parts and recoloured per vertex by tools/astronaut.py; no NASA endorsement implied | [NASA-3D-Resources](https://github.com/nasa/NASA-3D-Resources/tree/master/3D%20Models/Astronaut) | NASA 3D resources are free to download and use; credit NASA ([NASA media guidelines](https://www.nasa.gov/nasa-brand-center/images-and-media/)) |
| Curiosity rover 3D model (Walk mode: the rover parked at its recorded pose at each stop) | NASA/JPL-Caltech, NASA Science 3D resources, "Curiosity Rover 3D Model" (24584_Curiosity_static.glb); colours baked per vertex by tools/curiosity.py; no NASA endorsement implied | [model page](https://science.nasa.gov/resource/curiosity-rover-3d-model/) | NASA 3D resources are free to use; credit NASA ([NASA media guidelines](https://www.nasa.gov/nasa-brand-center/images-and-media/)) |
| MSL PLACES rover localizations | Deen, R. G. (2024), Mars Science Laboratory Rover PLACES Bundle (v42.0), NASA Planetary Data System, https://doi.org/10.17189/8nyv-8p60 | [localized_pos.csv](https://planetarydata.jpl.nasa.gov/img/data/msl/msl_places/data_localizations/localized_pos.csv) | PDS asks users to cite the bundle DOI ([PDS citing](https://pds.nasa.gov/datastandards/citing/)) |
| Curiosity Navcam images (one per stop) | NASA/JPL-Caltech (per image `image_credit`) | NASA raw-image feed, https://mars.nasa.gov/msl/multimedia/raw-images/ (image IDs and URLs in `assets/stops.json`) | [JPL image use policy](https://www.jpl.nasa.gov/jpl-image-use-policy); [NASA media guidelines](https://www.nasa.gov/nasa-brand-center/images-and-media/). The raw-image feed itself states no terms (not confirmed) |
| Curiosity Navcam processed data (sol 4718 XYZ, RNE and SLP products; drill-workspace point cloud in `assets/navcam_4718_points.bin`) | Maki, J. (2024), Mars Science Laboratory Navcam Processed Data Bundle, NASA PDS, https://doi.org/10.17189/2pw6-a555 | [msl_navcam_processed](https://planetarydata.jpl.nasa.gov/img/data/msl/msl_navcam_processed/DATA/) | as PDS above |
| Curiosity Navcam 360-degree mosaics (one per stop, `assets/pano_*.jpg`, product IDs in `assets/panos.json`) | NASA/JPL-Caltech; MSL Navcam mosaic products, NASA PDS, https://doi.org/10.17189/dk0p-1d15 | [MSLMOS_1XXX](https://planetarydata.jpl.nasa.gov/img/data/msl/MSLMOS_1XXX/DATA/) | as PDS above |
| CTX global mosaic, about 5 m (Mars view, streamed) | NASA/JPL/MSSS/The Caltech Murray Lab/Esri (CTX beta01, uncontrolled) | [Esri tile service](https://astro.arcgis.com/arcgis/rest/services/OnMars/CTX/MapServer); [Murray Lab](http://murray-lab.caltech.edu/CTX/) | "The Murray Lab/Caltech grants free use of the beta01 version of the mosaic for all purposes" (NASA Mars Trek layer description) |
| Viking VIS global colour mosaic and MGS MOLA global colour hillshade (Mars view, streamed) | via NASA Mars Trek (layers Mars_Viking_MDIM21_ClrMosaic_global_232m, Mars_MGS_MOLA_ClrShade_merge_global_463m) | [NASA Mars Trek](https://trek.nasa.gov/mars/) | Trek states no terms on the layer pages (not confirmed); NASA data, credit NASA Mars Trek |
| Chemistry along the walk (map layer, readout, per-station oxide tables) | ChemCam Multivariate Oxide Composition tables (Los Alamos National Laboratory; NASA/JPL-Caltech/LANL/CNES) and APXS oxide concentrations (NASA/JPL-Caltech/University of Guelph), MSL PDS release 42, placed at PLACES rover positions by tools/chem.py | [ChemCam MOC](https://pds-geosciences.wustl.edu/msl/msl-m-chemcam-libs-4_5-rdr-v1/mslccm_1xxx/data/moc/), [APXS](https://pds-geosciences.wustl.edu/msl/msl-m-apxs-4_5-rdr-v1/mslapx_1xxx/extras/) (PDS Geosciences Node) | PDS public data; credit the instrument teams; no terms-of-use statement found in the labels |
| Sun position check (Trust tab readiness row) | NASA Mars Trek sun-angle service (SPICE; NAIF 499 Mars, 10 Sun), queried by tools/trek_sun.py | [NASA Mars Trek](https://trek.nasa.gov/mars/) (endpoint https://trek.nasa.gov/getAzElfromT1) | as Trek above |
| Landing-site bookmarks (Mars view: site boxes, descriptions, images) | NASA Mars Trek bookmarks; Curiosity placed at PLACES site 1 instead of the box centre | [NASA Mars Trek catalogue](https://trek.nasa.gov/mars/) | as Trek above |

## Science sources for the stop cards

- NASA Curiosity blog: [Sols 4627-4628, a ridge stop in the boxworks](https://science.nasa.gov/blog/curiosity-blog-sols-4627-4628-a-ridge-stop-in-the-boxworks/);
  [Sols 4636-4637, up against a wall](https://science.nasa.gov/blog/curiosity-blog-sols-4636-4637-up-against-a-wall/);
  [Sols 4649-4654, ridges, hollows and nodules](https://science.nasa.gov/blog/curiosity-blog-sols-4649-4654-ridges-hollows-and-nodules-oh-my);
  [Sols 4689-4694, drill in the boxwork unit is go](https://science.nasa.gov/blog/curiosity-blog-sols-4689-4694-drill-in-the-boxwork-unit-is-go/);
  [Sols 4695-4701, searching for answers at Monte Grande](https://science.nasa.gov/solar-system/curiosity-blog-sols-4695-4701-searching-for-answers-at-monte-grande);
  [Sols 4709-4715, drilling high and low](https://science.nasa.gov/blog/curiosity-blog-sols-4709-4715-drilling-high-and-low-in-the-boxwork-unit);
  [Sols 4716-4722, drilling success at Nevado Sajama](https://science.nasa.gov/solar-system/curiosity-blog-sols-4716-4722-drilling-success-at-nevado-sajama).
- JPL news release, 23 Feb 2026: [NASA's Curiosity Rover Sees Martian Spiderwebs Up Close](https://www.jpl.nasa.gov/news/nasas-curiosity-rover-sees-martian-spiderwebs-up-close/).

## Method sources

- PLACES data products SIS (position accuracy, elevation source, roll/pitch/yaw conventions): [PLACES_PDS_SIS.pdf](https://planetarydata.jpl.nasa.gov/img/data/msl/msl_places/document/PLACES_PDS_SIS.pdf).
- MSL coordinate frames: [MSL_COORDINATE_FRAMES.PDF](https://pds-imaging.jpl.nasa.gov/documentation/MSL_COORDINATE_FRAMES.PDF).
- HiRISE DTM post spacing and precision: [UAHiRISE DTM about](https://www.uahirise.org/dtm/about.php).
- DTM slope error and resolution: Kirk, R. L. (2010), MSL landing-site workshop talk ([slides](https://marsoweb.nas.nasa.gov/landingsites/msl/workshops/4th_workshop/talks/3_Kirk_Slopes.pdf));
  Kirk et al. (2020), [ISPRS Archives XLIII-B3-2020](https://isprs-archives.copernicus.org/articles/XLIII-B3-2020/1129/2020/);
  Kirk et al. (2021), [ISPRS Archives XLIII-B3-2021](https://isprs-archives.copernicus.org/articles/XLIII-B3-2021/659/2021/).
- Rover tilt and mobility: Rankin, A. et al. (2020), Driving Curiosity: Mars Rover Mobility Trends During the First Seven Years ([JPL](https://www-robotics.jpl.nasa.gov/media/documents/2020-mobility-trending.pdf));
  Ali, K. S. et al. (2005), Attitude and position estimation on the Mars Exploration Rovers ([JPL](https://www-robotics.jpl.nasa.gov/media/documents/SAPP_paper.pdf));
  MSL landing press kit (2012), rover dimensions ([JPL](https://www.jpl.nasa.gov/news/press_kits/MSLLanding.pdf)).
- Walking-time penalty shape: Tobler, W. (1993), Three Presentations on Geographical Analysis and Modeling, NCGIA Technical Report 93-1 ([eScholarship](https://escholarship.org/uc/item/05r820mz)).
  Checked against the scanned report (Figure II, p. 4): "W = 6 exp { -3.5 * abs(S + 0.05)}", W in km/h, S = dh/dx = tan(theta);
  "On flat terrain this works out to 5 km/hr. For off-path travel multiply by 3/5". VISOR uses only the slope term exp(-3.5 tan(slope))
  on slope magnitude (no 0.05 downhill offset, no direction) as a relative penalty, scaled to an assumed 0.6 m/s on flat ground.
  Speed range: Norcross, J. R., et al. (2010), Metabolic Costs and Biomechanics of Level Ambulation in a Planetary Suit,
  NASA/TP-2010-216115 (0.4-0.8 m/s assumed for lunar intrasite translations).
- Route-planning practice (cost maps, keep-out zones, orbital vs stereo): JPL Mars 2020 traversability analysis
  ([PDF](https://www-robotics.jpl.nasa.gov/media/documents/Data-driven%20surface%20traversability%20analysis%20for%20Mars%202020%20landing%20site%20selection.pdf));
  JPL rover operations paper 2025 ([PDF](https://robotics.jpl.nasa.gov/media/documents/2025_IEEE_Aero_RO_ops.pdf)).

## Software

| Software | Used for | Licence (from each project; check before submission) |
| --- | --- | --- |
| Rust, `tiff` 0.9.1, `rayon` 1.12.0 | `crates/prep`, `crates/terrain-core` | tiff: MIT; rayon: MIT or Apache-2.0 |
| Bevy 0.20.1 | `app` (3D engine: rendering, input, scene) | MIT or Apache-2.0 |
| `serde` 1.0.229, `serde_json` 1.0.154 | `app`, `crates/route-core` (bundle JSON) | MIT or Apache-2.0 |
| `wasm-bindgen` 0.2.130, `web-sys` / `js-sys` 0.3.107, `wasm-bindgen-futures` 0.4.80, `console_error_panic_hook` 0.1.7 | `app`, `crates/plan-worker` (browser bindings) | MIT or Apache-2.0 |
| Trunk 0.21.14, wasm-opt (Binaryen) | web build | Trunk: MIT or Apache-2.0; Binaryen: Apache-2.0 |
| `leafwing-input-manager` 0.22.0 | `app` (held-key bindings) | MIT or Apache-2.0 |
| `leptos` 0.8.23 | `app` (HTML interface components) | MIT |
| Python 3.12, NumPy 2.2.1, SciPy 1.18.1, rasterio 1.5.2, Pillow 13.0.0 | `tools/` | NumPy, SciPy, rasterio: BSD-3-Clause; Pillow: MIT-CMU (HPND) |
| GDAL (bundled inside rasterio wheels) | reading the orthophoto COG | MIT |
| DracoPy 2.x | `tools/astronaut.py` (decoding the Draco-compressed NASA astronaut model) | Apache-2.0 |
| websocket-client | `tools/dev/` headless browser checks (Chrome DevTools protocol) | Apache-2.0 |

## AI assistance

Parts of the research, code and documents were produced with an AI assistant (Claude, Anthropic); see "Use of AI" in README.md.
The full AI-use disclosure is given in the Space Apps submission form.
