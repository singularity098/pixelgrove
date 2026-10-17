#!/usr/bin/env node
// Builds data/cities.tsv, the offline place-name table used to label photo locations.
//
// Source: GeoNames (https://www.geonames.org, CC BY 4.0) via the `all-the-cities` npm
// package. Keeps places with at least 15,000 inhabitants.
//
// Usage: npm install all-the-cities@3.1.0 && node scripts/build-cities.mjs > data/cities.tsv
// Columns: name, country, latitude, longitude (4 decimals ≈ 11 m), population.
import { createRequire } from 'module';

const require = createRequire(import.meta.url);
const cities = require('all-the-cities');
const countries = new Intl.DisplayNames(['en'], { type: 'region' });

const rows = cities
  .filter((c) => c.population >= 15000)
  .map((c) => {
    let country = c.country;
    try {
      country = countries.of(c.country) ?? c.country;
    } catch {
      // Unknown code: keep it as is.
    }
    const [lon, lat] = c.loc.coordinates;
    const clean = (s) => s.replace(/[\t\n]/g, ' ');
    return `${clean(c.name)}\t${clean(country)}\t${lat.toFixed(4)}\t${lon.toFixed(4)}\t${c.population}`;
  })
  .sort();
process.stdout.write(rows.join('\n') + '\n');
