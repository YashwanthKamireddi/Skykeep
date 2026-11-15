/**
 * Shared Albums, as icloud.com's Sharing section lists them: a card each, with its newest photo,
 * its name, who made it ("Created by You", or one it was invited to) and when Apple ends it.
 * Read from the library, which each sync keeps as Apple has them (iOS 27's shared collections);
 * a card opens the album in the grid, to look through, copy and save. Making albums, adding to
 * them and comments come once Apple's ways of writing them are mapped.
 */
import { t } from "../../lib/i18n";
import { dateLocale } from "../../lib/when";
import { EmptyState } from "../../components/EmptyState";
import { PaneTitle } from "../../components/PaneTitle";
import { useEffect, useState } from "react";
import { Users } from "lucide-react";
import { api, pictureSrc, type SharedAlbum } from "../../lib/ipc";
import { useRevision } from "../../lib/sync";

/** When an album ends, as icloud.com says it: "Expires November 19". */
function expiry(at: number | null): string | null {
  if (!at) return null;
  return t("photos.expires", { date: new Date(at * 1000).toLocaleDateString(dateLocale(), { month: "long", day: "numeric" }) });
}

/** An album's picture: its cover, or — none, or one that won't load (offline, a link Apple let
 *  lapse) — the album's plain glyph, never a broken image. */
function Cover({ id }: { id: string | null }) {
  const [failed, setFailed] = useState(false);
  if (!id || failed) return <Users size={34} strokeWidth={1.6} aria-hidden="true" />;
  // A few covers, asked for at once: lazily, each came a frame or two after its card (grey first).
  return <img src={pictureSrc(id, "thumb")} alt="" loading="eager" decoding="async" draggable={false} onError={() => setFailed(true)} />;
}

export function SharedAlbums({ onError, onOpen }: { onError: (message: string) => void; onOpen: (album: SharedAlbum) => void }) {
  const [albums, setAlbums] = useState<SharedAlbum[] | null>(null);
  // Read again whenever a sync changed Photos: an album new, changed or gone redraws the page.
  const revision = useRevision("photos");

  useEffect(() => {
    api.photosSharedAlbums().then(setAlbums, (e) => {
      setAlbums((was) => was ?? []);
      onError(String(e));
    });
  }, [revision]); // eslint-disable-line react-hooks/exhaustive-deps

  const n = albums?.length ?? 0;
  return (
    <div className="px-albums-page">
      <header className="px-albums-head">
        <PaneTitle className="px-heading" countClassName="px-span" count={albums === null ? " " : n === 1 ? "1 Shared Album" : `${n} Shared Albums`}>
          {t("photos.sharedAlbums")}
        </PaneTitle>
      </header>
      {albums !== null && n === 0 ? (
        <EmptyState className="px-empty px-shared-empty" icon={<Users size={28} strokeWidth={1.6} />}>
          {t("photos.sharedAlbumsEmpty")}
        </EmptyState>
      ) : (
        <div className="px-cards" role="list" aria-busy={albums === null}>
          {(albums ?? []).map((a) => {
            const name = a.title ?? t("photos.sharedAlbum");
            const ends = expiry(a.expiresAt);
            return (
              <button key={a.zone} type="button" role="listitem" className="px-card" aria-label={name} onClick={() => onOpen(a)}>
                <span className="px-card-pic">
                  <Cover key={a.cover ?? ""} id={a.cover} />
                </span>
                <span className="px-card-name">{name}</span>
                <span className="px-card-count">{a.database === "private" ? "Created by You" : "Shared with You"}</span>
                {ends && <span className="px-card-count">{ends}</span>}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
