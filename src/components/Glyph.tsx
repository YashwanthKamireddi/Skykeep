import {
  Anchor,
  Baby,
  Backpack,
  Bell,
  Bike,
  BookMarked,
  Bookmark,
  BookOpen,
  Briefcase,
  Building2,
  Bus,
  Cake,
  Camera,
  Car,
  Cat,
  Cloud,
  CloudRain,
  CreditCard,
  Dog,
  Film,
  Flower2,
  Gamepad2,
  Gift,
  Globe,
  GraduationCap,
  Hammer,
  Heart,
  HeartPulse,
  Key,
  Laptop,
  Leaf,
  Library,
  MapPin,
  Music,
  PartyPopper,
  PawPrint,
  Pencil,
  PiggyBank,
  Pill,
  Plane,
  Printer,
  Puzzle,
  ShoppingBag,
  ShoppingCart,
  Snowflake,
  Sofa,
  Sparkles,
  Star,
  Tag,
  TrainFront,
  TreePalm,
  TreePine,
  Trophy,
  Tv,
  Umbrella,
  User,
  Users,
  Utensils,
  Volleyball,
  Wallet,
  Wine,
  Wrench,
  Zap,
  Calendar,
  Check,
  CalendarDays,
  ChevronRight,
  Dumbbell,
  Flag,
  Folder,
  House,
  Inbox,
  Info,
  Lightbulb,
  List,
  Plus,
  Search,
  Sun,
  type LucideIcon,
} from "lucide-react";

/** The icons the app's screens mark things with, by what they mean rather than how drawn. */
export type GlyphName =
  | "today"
  | "scheduled"
  | "all"
  | "flagged"
  | "completed"
  | "list"
  | "bullets"
  | "fitness"
  | "education"
  | "home"
  | "idea"
  | "sun"
  | "folder"
  | "plus"
  | "info"
  | "chevron"
  | "search";


/**
 * Each glyph from one professionally drawn set, on one grid and one stroke — the nearest
 * open equivalents of the SF Symbols Apple's apps use, which cannot be shipped outside
 * Apple's platforms. The hand-drawn paths these replaced read as rough beside Apple's.
 */
const ICONS: Record<GlyphName, { icon: LucideIcon; filled?: boolean }> = {
  today: { icon: Calendar },
  scheduled: { icon: CalendarDays },
  all: { icon: Inbox },
  flagged: { icon: Flag, filled: true },
  completed: { icon: Check },
  list: { icon: List },
  bullets: { icon: List },
  fitness: { icon: Dumbbell },
  education: { icon: BookOpen },
  home: { icon: House },
  idea: { icon: Lightbulb },
  sun: { icon: Sun },
  folder: { icon: Folder },
  plus: { icon: Plus },
  info: { icon: Info },
  chevron: { icon: ChevronRight },
  search: { icon: Search },
};

/**
 * Apple's list icons by family, numbered within each (`shopping1` … `shopping4`): every family
 * the Reminders web app offers (docs/icloud-web/reminders.md, BadgeEmblem). Apple draws them
 * with SF Symbols, which can't be shipped outside its platforms, so each number gets the open
 * glyph nearest its family's meaning, different numbers different glyphs, so two lists with
 * two icons from one family still look apart.
 */
const EMBLEMS: Record<string, LucideIcon[]> = {
  bookmarks: [Bookmark, BookMarked],
  celebration: [PartyPopper, Cake],
  education: [BookOpen, GraduationCap, Pencil, Backpack, Library],
  finance: [Wallet, CreditCard, PiggyBank],
  fitness: [Dumbbell],
  sport: [Trophy, Bike, Volleyball, Dumbbell, Trophy, Bike],
  food: [Utensils],
  wine: [Wine],
  health: [HeartPulse, Pill],
  lifestyle: [House, Sofa],
  location: [MapPin, Building2, Globe],
  media: [Music, Film, Tv, Gamepad2, Camera],
  nature: [Leaf, TreePine, Flower2],
  people: [Users, User, Baby],
  pet: [PawPrint, Dog, Cat],
  shopping: [ShoppingCart, ShoppingBag, Gift, Tag],
  transport: [Car, Plane, TrainFront, Bus],
  vacation: [TreePalm],
  weather: [Sun, Cloud, CloudRain, Snowflake, Umbrella],
  work: [Briefcase, Laptop, Hammer, Wrench, Printer],
  concept: [Lightbulb, Puzzle, Sparkles],
  symbol: [Star, Heart, Flag, Bell, Key, Anchor, Zap],
};

/** A list's emoji, when its icon is one: Apple keeps it as `{"Emoji":"🛒"}`. */
export function emblemEmoji(emblem: string | null | undefined): string | null {
  if (!emblem || !emblem.trimStart().startsWith("{")) return null;
  try {
    const emoji = (JSON.parse(emblem) as { Emoji?: unknown }).Emoji;
    return typeof emoji === "string" && emoji.trim() ? emoji : null;
  } catch {
    return null;
  }
}

/** The glyph for one of Apple's named list icons; the plain bulleted list for one not known. */
function emblemIcon(emblem: string | null | undefined): LucideIcon {
  const name = (emblem ?? "").trim().toLowerCase();
  const match = /^([a-z]+)(\d*)$/.exec(name);
  const family = match ? EMBLEMS[match[1]] : undefined;
  if (!family) return List;
  const n = match?.[2] ? Number(match[2]) : 1;
  return family[(n - 1) % family.length] ?? family[0];
}

/**
 * A list's own icon, as the person chose it on their iPhone: its emoji, or the glyph for
 * Apple's named icon. Before, only five families were known and every emoji was dropped, so
 * most lists showed the same plain icon (#40).
 */
export function ListEmblem({ emblem }: { emblem: string | null | undefined }) {
  const emoji = emblemEmoji(emblem);
  if (emoji) {
    return (
      <span className="list-emoji" aria-hidden="true">
        {emoji}
      </span>
    );
  }
  const Icon = emblemIcon(emblem);
  return <Icon size={16} strokeWidth={2.1} fill="none" aria-hidden="true" focusable="false" />;
}

export function Glyph({ name, className }: { name: GlyphName; className?: string }) {
  const { icon: Icon, filled } = ICONS[name];
  return (
    <Icon
      className={className}
      size={16}
      strokeWidth={2.1}
      absoluteStrokeWidth={false}
      fill={filled ? "currentColor" : "none"}
      aria-hidden="true"
      focusable="false"
    />
  );
}
