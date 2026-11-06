import { FITS, fitSrc, type Photo } from "./ipc";

// This bounds the requested decoded pixels of grid images, not WebKit's internal cache.
export const GRID_IMAGE_BYTES = 32 * 1024 * 1024;
type TileImage = { image: HTMLImageElement; photo: Photo; wanted: number; near: boolean };
const tiles = new Map<HTMLImageElement, TileImage>();
let observer: IntersectionObserver | null = null;
let frame = 0;

function release(image: HTMLImageElement) {
  image.removeAttribute("src");
  if (image.parentElement) image.parentElement.style.backgroundImage = "";
}

function update() {
  frame = 0;
  const near = [...tiles.values()].filter((t) => t.near);
  // Keep visible images first if the viewport is exceptionally large.
  const center = window.innerHeight / 2;
  near.sort((a, b) => Math.abs(a.image.getBoundingClientRect().top - center) - Math.abs(b.image.getBoundingClientRect().top - center));
  let remaining = GRID_IMAGE_BYTES;
  near.forEach((tile, index) => {
    const edge = Math.min(512, tile.wanted, Math.sqrt(remaining / (near.length - index) / 4));
    const size = [...FITS].reverse().find((s) => s <= edge);
    if (!size) {
      release(tile.image);
      return;
    }
    // Square accounting is conservative for portrait and landscape images alike.
    remaining -= size * size * 4;
    const src = fitSrc(tile.photo, size);
    if (tile.image.getAttribute("src") !== src) tile.image.src = src;
  });
}

function schedule() {
  if (!frame) frame = requestAnimationFrame(update);
}

export function watchTileImage(image: HTMLImageElement, photo: Photo, wanted: number): () => void {
  observer ??= new IntersectionObserver((entries) => {
    for (const entry of entries) {
      const tile = tiles.get(entry.target as HTMLImageElement);
      if (!tile) continue;
      tile.near = entry.isIntersecting;
      if (!tile.near) release(tile.image);
    }
    schedule();
  }, { rootMargin: "160px 0px" });
  tiles.set(image, { image, photo, wanted: Math.max(128, wanted), near: false });
  observer.observe(image);
  return () => {
    observer?.unobserve(image);
    tiles.delete(image);
    release(image);
    if (!tiles.size) {
      observer?.disconnect();
      observer = null;
      cancelAnimationFrame(frame);
      frame = 0;
    } else schedule();
  };
}
