// three.js is only published as an ES module now, and the rest of the page is plain scripts:
// this puts it where flightmap.js looks and says when it is there.
import * as THREE from '../vendor/three/three.module.min.js';

self.THREE = THREE;
self.dispatchEvent(new Event('three'));
