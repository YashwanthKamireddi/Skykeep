// The arithmetic Calendar's views are drawn from, checked directly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { placeDay, placeDayWith, weekSpans, monthDays, daysOf, snap, clock, startOfWeek, weekInfoOf, hourLabel, endAfter } from "../src/screens/calendar/model.ts";

const at = (y, m, d, h = 0, min = 0) => Math.floor(new Date(y, m, d, h, min).getTime() / 1000);
const ev = (guid, startsAt, endsAt, allDay = false) =>
  ({ guid, calendarGuid: "c", title: guid, location: null, startsAt, endsAt, allDay, timezone: null, url: null, notes: null, etag: null });

test("an end set before the start runs into the next day, on the event's own clock", () => {
  const z = "Asia/Kolkata";
  const start = instantIn(z, { y: 2026, m: 11, d: 11, h: 23, min: 0 });
  // 1:00 typed for a 23:00 event: 1:00 the next morning, not 23:05.
  const end = endAfter(z, start, instantIn(z, { y: 2026, m: 11, d: 11, h: 1, min: 0 }));
  assert.equal(end, instantIn(z, { y: 2026, m: 11, d: 12, h: 1, min: 0 }));
  // Across a clock change the next day's 1:00 is still 1:00 on that clock.
  const ny = "America/New_York";
  const late = instantIn(ny, { y: 2026, m: 10, d: 31, h: 23, min: 30 });
  assert.equal(endAfter(ny, late, instantIn(ny, { y: 2026, m: 10, d: 31, h: 1, min: 0 })), instantIn(ny, { y: 2026, m: 11, d: 1, h: 1, min: 0 }));
  // An end after the start stays; one still before it a day on, and an end equal to the
  // start, are five minutes after it, as before.
  assert.equal(endAfter(z, start, start + 3600), start + 3600);
  assert.equal(endAfter(z, start, start - 3 * 86_400), start + 300);
  assert.equal(endAfter(z, start, start), start + 300);
});

test("a month is six weeks from the Sunday on or before the 1st", () => {
  const days = monthDays(new Date(2026, 8, 15), 0);
  assert.equal(days.length, 42);
  assert.equal(days[0].getDay(), 0);
  assert.deepEqual([days[0].getMonth(), days[0].getDate()], [7, 30]);
});

test("an all-day event ends the day before its end midnight", () => {
  const [first, last] = daysOf(ev("a", at(2026, 9, 2), at(2026, 9, 4), true));
  assert.equal(first.getDate(), 2);
  assert.equal(last.getDate(), 3);
});

test("a trip across a weekend is one bar per week, carrying on across the break", () => {
  const trip = ev("trip", at(2026, 9, 2), at(2026, 9, 6), true); // Fri 2 – Mon 5
  const first = weekSpans([trip], new Date(2026, 8, 27));
  const second = weekSpans([trip], new Date(2026, 9, 4));
  assert.deepEqual([first[0].from, first[0].to, first[0].continuesRight], [5, 6, true]);
  assert.deepEqual([second[0].from, second[0].to, second[0].continuesLeft], [0, 1, true]);
});

test("overlapping bars take separate lanes, and a free lane is reused", () => {
  const week = new Date(2026, 8, 27);
  const spans = weekSpans([
    ev("long", at(2026, 8, 27), at(2026, 9, 1), true),
    ev("mid", at(2026, 8, 28), at(2026, 8, 30), true),
    ev("late", at(2026, 9, 1), at(2026, 9, 3), true),
  ], week);
  const lane = Object.fromEntries(spans.map((s) => [s.event.guid, s.lane]));
  assert.equal(lane.long, 0);
  assert.equal(lane.mid, 1);
  assert.equal(lane.late, 0);
});

test("overlapping meetings sit side by side; a later one gets the width back", () => {
  const day = new Date(2026, 8, 29);
  const placed = placeDay([
    ev("a", at(2026, 8, 29, 9), at(2026, 8, 29, 10)),
    ev("b", at(2026, 8, 29, 9, 30), at(2026, 8, 29, 11)),
    ev("c", at(2026, 8, 29, 14), at(2026, 8, 29, 15)),
  ], day);
  const by = Object.fromEntries(placed.map((p) => [p.event.guid, p]));
  assert.deepEqual([by.a.column, by.a.columns], [0, 2]);
  assert.deepEqual([by.b.column, by.b.columns], [1, 2]);
  assert.deepEqual([by.c.column, by.c.columns], [0, 1]);
  assert.equal(by.a.top, 540);
});

test("a reminder due during a meeting takes a column beside it", () => {
  const day = new Date(2026, 8, 29);
  const { events, marks } = placeDayWith([ev("m", at(2026, 8, 29, 19), at(2026, 8, 29, 20))], day,
    [{ id: "r", top: 19 * 60, length: 22.5 }, { id: "late", top: 21 * 60, length: 22.5 }]);
  assert.deepEqual([events[0].column, events[0].columns], [0, 2]);
  const by = Object.fromEntries(marks.map((m) => [m.id, m]));
  assert.deepEqual([by.r.column, by.r.columns], [1, 2]);
  assert.deepEqual([by.late.column, by.late.columns], [0, 1]);
});

test("a five-minute call is still a quarter hour tall", () => {
  const [p] = placeDay([ev("x", at(2026, 8, 29, 9), at(2026, 8, 29, 9, 5))], new Date(2026, 8, 29));
  assert.equal(p.length, 15);
});

test("an evening that runs past midnight is a block on each day it touches", () => {
  const late = ev("late", at(2026, 8, 29, 23), at(2026, 8, 30, 1));
  const [tue] = placeDay([late], new Date(2026, 8, 29));
  const [wed] = placeDay([late], new Date(2026, 8, 30));
  assert.deepEqual([tue.top, tue.length], [23 * 60, 60]);
  assert.deepEqual([wed.top, wed.length], [0, 60]);
  assert.equal(weekSpans([late], new Date(2026, 8, 27)).length, 0);
});

test("a timed event of a day or more is a bar", () => {
  const conference = ev("conf", at(2026, 8, 29, 9), at(2026, 9, 1, 17));
  assert.equal(placeDay([conference], new Date(2026, 8, 30)).length, 0);
  assert.equal(weekSpans([conference], new Date(2026, 8, 27))[0].to, 4);
});

test("dragging snaps to the quarter hour", () => {
  assert.equal(snap(7), 0);
  assert.equal(snap(8), 15);
  assert.equal(snap(52), 45);
});

test("a time on the hour leaves off the minutes", () => {
  assert.doesNotMatch(clock(at(2026, 8, 29, 15), true), /:00/);
  assert.match(clock(at(2026, 8, 29, 15, 30), true), /30/);
});

import { wallIn, instantIn, sameClockIn, zoneName } from "../src/screens/calendar/model.ts";
const Z = (y, m, d, h, min) => Date.UTC(y, m - 1, d, h, min) / 1000;
test("a clock reading in any zone, and back", () => {
  assert.deepEqual(wallIn("America/New_York", Z(2026, 10, 21, 13, 0)), { y: 2026, m: 10, d: 21, h: 9, min: 0 });
  assert.equal(instantIn("America/New_York", { y: 2026, m: 10, d: 21, h: 9, min: 0 }), Z(2026, 10, 21, 13, 0));
  assert.equal(instantIn("Asia/Kolkata", { y: 2026, m: 10, d: 21, h: 9, min: 0 }), Z(2026, 10, 21, 3, 30));
});
test("a zone changed keeps the clock: 9:00 in Kolkata becomes 9:00 in New York", () => {
  assert.equal(sameClockIn("Asia/Kolkata", "America/New_York", Z(2026, 10, 21, 3, 30)), Z(2026, 10, 21, 13, 0));
});
test("across a clock change: a skipped time an hour on, a doubled one the first time", () => {
  // 2:30 on 8 March 2026 never shows in New York: 3:30 EDT.
  assert.equal(instantIn("America/New_York", { y: 2026, m: 3, d: 8, h: 2, min: 30 }), Z(2026, 3, 8, 7, 30));
  // 1:30 on 1 November 2026 shows twice: the first, still EDT.
  assert.equal(instantIn("America/New_York", { y: 2026, m: 11, d: 1, h: 1, min: 30 }), Z(2026, 11, 1, 5, 30));
});
test("a zone named as Calendar names it", () => {
  assert.deepEqual(zoneName("Asia/Kolkata", Z(2026, 10, 21, 0, 0)), { city: "Kolkata", offset: "GMT+5:30" });
  assert.deepEqual(zoneName("America/New_York", Z(2026, 10, 21, 0, 0)), { city: "New York", offset: "GMT−4" });
  assert.deepEqual(zoneName("UTC", Z(2026, 10, 21, 0, 0)).offset, "GMT");
});
import { addDaysIn } from "../src/screens/calendar/model.ts";
test("a day on is a calendar day, across a clock change too", () => {
  // Fri 30 Oct 2026, all day in New York, a week on: Fri 6 Nov at midnight, not Thu at 23:00.
  const fri = Z(2026, 10, 30, 4, 0);
  assert.equal(addDaysIn("America/New_York", fri, 7), Z(2026, 11, 6, 5, 0));
  assert.equal(addDaysIn("Asia/Kolkata", Z(2026, 10, 21, 3, 30), 1), Z(2026, 10, 22, 3, 30));
  assert.equal(addDaysIn("America/New_York", Z(2026, 10, 31, 4, 0), 1), Z(2026, 11, 1, 4, 0));
});

test("a week starts where the region starts it, and a 24-hour clock keeps its :00", () => {
  // Monday first: September 2026 opens on Monday 31 August.
  const mon = monthDays(new Date(2026, 8, 15), 1);
  assert.equal(mon[0].getDay(), 1);
  assert.deepEqual([mon[0].getMonth(), mon[0].getDate()], [7, 31]);
  // A Sunday belongs to the week before it when weeks start on Monday.
  assert.equal(startOfWeek(new Date(2026, 9, 25), 1).getDate(), 19);
  assert.equal(startOfWeek(new Date(2026, 9, 25), 0).getDate(), 25);
  assert.equal(weekInfoOf("en-GB").firstDay, 1);
  assert.equal(weekInfoOf("en-US").firstDay, 0);
  assert.deepEqual(weekInfoOf("de-DE").weekend.sort(), [0, 6]);
  assert.match(clock(at(2026, 8, 29, 15), false), /:00/);
  assert.notEqual(hourLabel(12, false), "Noon");
  assert.equal(hourLabel(12, true), "Noon");
});

test("week numbers: ISO where weeks start on Monday, the week of 1 January elsewhere", async () => {
  const { weekNumber } = await import("../src/screens/calendar/model.ts");
  // ISO: 29 Dec 2025 (Mon) is week 1 of 2026; 28 Dec 2026 is week 53; 19 Oct 2026 is 43.
  assert.equal(weekNumber(new Date(2025, 11, 29), 1), 1);
  assert.equal(weekNumber(new Date(2026, 9, 19), 1), 43);
  assert.equal(weekNumber(new Date(2026, 11, 28), 1), 53);
  // US: the week of Sun 28 Dec 2025 holds 1 Jan, so is week 1; 18 Oct 2026 is week 43.
  assert.equal(weekNumber(new Date(2025, 11, 28), 0), 1);
  assert.equal(weekNumber(new Date(2026, 9, 18), 0), 43);
  assert.equal(weekNumber(new Date(2026, 11, 27), 0), 1);
});
