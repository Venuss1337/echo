/** Seed only the newly created private test profile with an inert, muted queue. */
import { Database } from "bun:sqlite";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";

/** The published copy of `tests/fixtures/audio` used by fixture radio tests. */
const fixtureOrigin =
  "https://raw.githubusercontent.com/Venuss1337/echo/v1.0.0/tests/fixtures/audio";

const profile = resolve(process.argv[2]);
const audio = resolve(process.argv[3]);
if (existsSync(profile) || !existsSync(audio))
  throw new Error("A fresh profile and generated audio fixture are required.");
// Verify the remote fixtures before creating anything.
const origin = (process.env.ECHO_RADIO_FIXTURE_ORIGIN ?? fixtureOrigin).replace(
  /\/+$/,
  "",
);
if (process.argv[4] === "fixture") {
  // The app keeps its full production URL policy, so the stations must be
  // public HTTPS. They serve this repository's own committed test
  // encodings. ECHO_RADIO_FIXTURE_ORIGIN can point at a fork or a pinned
  // commit; whatever the origin, every file is compared byte for byte with
  // the checkout before the test may rely on it, so a moved tag or another
  // host can never silently change what the radio test plays.
  if (!origin.startsWith("https://"))
    throw new Error("The radio fixture origin must use HTTPS.");
  for (const codec of ["mp3", "aac"]) {
    const expected = readFileSync(`tests/fixtures/audio/tone.${codec}`);
    const response = await fetch(`${origin}/tone.${codec}`);
    const served = new Uint8Array(await response.arrayBuffer());
    if (!response.ok || !Buffer.from(served).equals(expected))
      throw new Error(
        `${origin}/tone.${codec} does not serve the committed fixture.`,
      );
  }
}
mkdirSync(profile);
const database = new Database(join(profile, "library.sqlite3"));
try {
  const directory = "src-tauri/migrations";
  database.transaction(() => {
    readdirSync(directory)
      .filter((name) => /^\d{3}_.*\.sql$/.test(name))
      .sort()
      .forEach((name, index) => {
        database.exec(readFileSync(join(directory, name), "utf8"));
        database.exec(`PRAGMA user_version = ${index + 1}`);
      });
    database
      .query("INSERT INTO playback_session (id, payload) VALUES (1, ?1)")
      .run(
        JSON.stringify({
          version: 1,
          queue: {
            entries: [{ id: 1, path: audio, track: null }],
            order: [1],
            current: 1,
            last_id: 1,
            shuffled: false,
            repeat: "Off",
          },
          volume: 34,
          muted: true,
          position: 0,
        }),
      );
    if (process.argv[4] === "fixture") {
      const fixtureIds = ["mp3", "aac"].map((codec, index) => {
        const id = `local:${String(index + 1).padStart(32, "0")}`;
        database
          .query(
            "INSERT INTO radio_stations (id, name, stream_url, country, language, tags, codec, bitrate, manual, updated_at) VALUES (?1, ?2, ?3, '', '', '', ?4, 0, 1, unixepoch())",
          )
          .run(
            id,
            `Fixture ${codec.toUpperCase()}`,
            `${origin}/tone.${codec}`,
            codec.toUpperCase(),
          );
        return id;
      });
      const query = {
        name: "",
        country: "",
        language: "",
        tag: "",
        sort: "popular",
        offset: 0,
      };
      database
        .query("INSERT INTO radio_directory_cache VALUES (?1, ?2, unixepoch())")
        .run(JSON.stringify(query), JSON.stringify([fixtureIds, false]));
      if (process.argv[5] === "upgrade") {
        database.run("INSERT INTO library_roots (id, path) VALUES (1, ?)", [
          dirname(audio),
        ]);
        database.exec(
          "INSERT INTO albums (id, title, title_key, key) VALUES (1, 'Fixture album', 'fixture album', 'fixture album')",
        );
        database.run(
          `INSERT INTO tracks (id, root_id, rel_path, path_key, size, mtime_ns, title,
          title_key, artist_text, album_id, duration_ms, codec, metadata_version)
          VALUES (1, 1, ?, ?, ?, 0, 'Fixture track', 'fixture track', 'Fixture artist', 1, 12000, 'WAV', 1)`,
          [
            basename(audio),
            basename(audio).toLowerCase(),
            statSync(audio).size,
          ],
        );
        database.run(
          `INSERT INTO media_sources (id, track_id, last_track_id, abs_key, rel_key,
          size, mtime_ns, title, artist, album, duration_ms)
          VALUES (1, 1, 1, ?, ?, ?, 0, 'Fixture track', 'Fixture artist', 'Fixture album', 12000)`,
          [
            audio.toLowerCase(),
            basename(audio).toLowerCase(),
            statSync(audio).size,
          ],
        );
        database.exec(`
          INSERT INTO playlists VALUES (1, 'Upgrade fixture list', '', 1, 1);
          INSERT INTO playlist_entries VALUES (1, 1, 1, 1, 1);
          INSERT INTO favorite_tracks VALUES (1, 1);
          INSERT INTO favorite_albums VALUES (1, 'fixture album', 'Fixture album', 'Fixture artist', NULL, 1);
          INSERT INTO plays VALUES (1, 1, 'fixture album', 1, 6000);
          INSERT INTO radio_preferences VALUES (1, 'name');
        `);
        database.run(
          "UPDATE radio_stations SET favorite = 1, heard_at = unixepoch() WHERE id = ?",
          [fixtureIds[0]],
        );
      }
    }
  })();
} finally {
  database.close();
}
