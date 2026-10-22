import type { Book } from "../../lib/notes";
import type { Board } from "../../lib/todo";
import type { Inbox, Letter, MailView } from "../../lib/mail";
import type { Codes, View as TotpView } from "../../lib/totp";
import { DAY, HOUR, MINUTE, NOW, type Answers } from "./common";

/* Текст вымышленный; даты писем отсчитываются от общего «сейчас». */

const NOTES: Book & { session: number } = {
  pages: [
    "A little space for ideas. Keep the next step small, leave room for a useful detail, and return to the draft with fresh eyes. Today is a good day to turn one idea into something tangible.",
    "Project notes\n\nReview the layout, collect a few references, and keep the essentials close.\nhttps://example.com/guide",
    "Ideas for later\n\nA reading list, a weekend project, and a new playlist.",
  ],
  active: 0,
  session: 1,
};

const TODO: Board & { session: number } = {
  lists: [[
    { id: 1, text: "Outline the next project", done: true },
    { id: 2, text: "Collect a few references", done: true },
    { id: 3, text: "Review the first draft", done: false },
    { id: 4, text: "Take a short walk", done: false },
    { id: 5, text: "Organize the workspace", done: false },
    { id: 6, text: "Read one chapter", done: false },
  ]],
  active: 0,
  session: 1,
};

function letter(account: string, uid: number, from: string, fromAddress: string,
  subject: string, preview: string, age: number, seen: boolean): Letter {
  return {
    account,
    revision: "demo-revision",
    uidValidity: 1,
    uid,
    from,
    fromAddress,
    subject,
    preview,
    at: Math.floor((NOW - age) / 1000),
    seen,
    link: null,
    to: [account],
  };
}

const LETTERS: Letter[] = [
  letter("demo@example.test", 105, "Project Desk", "team@example.test",
    "The first draft is ready", "The updated outline and a short review checklist are ready.", 12 * MINUTE, false),
  letter("demo@example.test", 104, "Reading Club", "reading@example.test",
    "This week's reading list", "Three new articles and a thoughtful question for the weekend.", 48 * MINUTE, false),
  letter("demo@example.test", 103, "Design Notes", "notes@example.test",
    "A few ideas for the next iteration", "A simpler layout and more room for the essential details.", 3 * HOUR, true),
  letter("demo@example.test", 102, "Workshop Updates", "workshop@example.test",
    "Workshop materials are available", "The practice files and a short recap are ready to explore.", 6 * HOUR, false),
  letter("demo@example.test", 101, "Example Digest", "digest@example.test",
    "Your weekly collection", "A quiet collection of useful links, notes, and small discoveries.", DAY, true),
];

const ALERTS: Letter[] = [
  letter("alerts@example.test", 101, "Build Monitor", "builds@example.test",
    "Demo build completed", "All checks passed. The sample build is ready for review.", 2 * HOUR, true),
];

function inbox(letters: Letter[]): Inbox {
  return {
    uidValidity: 1,
    unseen: letters.filter((entry) => !entry.seen).length,
    letters,
    mailbox: null,
  };
}

const MAIL: MailView = {
  accounts: [
    { id: "demo@example.test", revision: "demo-revision", address: "demo@example.test",
      view: { status: "ready", stale: false, inbox: inbox(LETTERS) } },
    { id: "alerts@example.test", revision: "demo-revision", address: "alerts@example.test",
      view: { status: "ready", stale: false, inbox: inbox(ALERTS) } },
  ],
  preview: true,
};

const TOTP: TotpView = {
  accounts: [
    { id: "github", issuer: "GitHub", label: "demo@example.test", digits: 6, period: 30 },
    { id: "google", issuer: "Google", label: "demo@example.test", digits: 6, period: 30 },
    { id: "discord", issuer: "Discord", label: "demo@example.test", digits: 6, period: 30 },
    { id: "example", issuer: "Example", label: "demo@example.test", digits: 6, period: 30 },
  ],
  trouble: null,
};

/** Срок обновляется на каждый вопрос, иначе карточка просила бы истёкший код. */
function totpCodes(ids: unknown): Codes {
  const now = Math.floor(Date.now() / 1000);
  const codes: Codes = {
    now,
    codes: TOTP.accounts.map((row, index) => ({
      id: row.id,
      code: String(482915 + index * 137045),
      validUntil: (Math.floor(now / row.period) + 1) * row.period,
    })).filter((row) => !Array.isArray(ids) || ids.length === 0 || ids.includes(row.id)),
    clockSuspect: false,
  };
  return codes;
}

export const text: Answers = {
  notes_pages: () => NOTES,
  todo_lists: () => TODO,
  mail_state: () => MAIL,
  totp_state: () => TOTP,
  totp_codes: (args) => totpCodes(args.ids),
};
