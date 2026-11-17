-- Courses: what we teach.
CREATE TABLE courses (
    id            INTEGER PRIMARY KEY,
    code          TEXT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    description   TEXT,
    level         TEXT,
    status        TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft','published')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);

-- Classes: one run of a course.
CREATE TABLE classes (
    id            INTEGER PRIMARY KEY,
    course_id     INTEGER NOT NULL REFERENCES courses(id),
    code          TEXT NOT NULL UNIQUE,
    teacher       TEXT,
    term_start    TEXT NOT NULL,                 -- YYYY-MM-DD
    term_end      TEXT NOT NULL,                 -- YYYY-MM-DD
    timezone      TEXT NOT NULL DEFAULT 'UTC',   -- IANA
    capacity      INTEGER NOT NULL CHECK (capacity > 0),
    fee_cents     INTEGER NOT NULL DEFAULT 0 CHECK (fee_cents >= 0),  -- USD cents
    status        TEXT NOT NULL DEFAULT 'draft'
                  CHECK (status IN ('draft','open','closed','cancelled')),
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    CHECK (term_end >= term_start)
);
CREATE INDEX idx_classes_course ON classes(course_id);
CREATE INDEX idx_classes_public ON classes(status, term_start);

-- Weekly repeating sessions.
CREATE TABLE class_sessions (
    id            INTEGER PRIMARY KEY,
    class_id      INTEGER NOT NULL REFERENCES classes(id) ON DELETE CASCADE,
    day_of_week   INTEGER NOT NULL CHECK (day_of_week BETWEEN 1 AND 7), -- 1 = Monday
    start_time    TEXT NOT NULL,                 -- HH:MM in classes.timezone
    end_time      TEXT NOT NULL,
    CHECK (end_time > start_time)
);
CREATE INDEX idx_class_sessions_class ON class_sessions(class_id);

CREATE TABLE bookings (
    id               INTEGER PRIMARY KEY,
    reference        TEXT NOT NULL UNIQUE,
    cancel_code_hash TEXT NOT NULL,
    class_id         INTEGER NOT NULL REFERENCES classes(id),
    name             TEXT,                       -- NULL after retention clean-up
    email            TEXT,                       -- lower-case; NULL after clean-up
    status           TEXT NOT NULL DEFAULT 'pending'
                     CHECK (status IN ('pending','confirmed','rejected','cancelled','expired')),
    pending_until    TEXT NOT NULL,              -- RFC 3339 UTC
    decided_by       TEXT,
    decided_at       TEXT,
    reason           TEXT,
    client_ip        TEXT,
    created_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
CREATE INDEX idx_bookings_class_status ON bookings(class_id, status);
CREATE INDEX idx_bookings_email_status ON bookings(email, status);
CREATE UNIQUE INDEX uq_bookings_active_email
    ON bookings(class_id, email) WHERE status IN ('pending','confirmed');

CREATE TABLE revoked_tokens (
    jti           TEXT PRIMARY KEY,
    sub           TEXT NOT NULL,
    expires_at    INTEGER NOT NULL,
    revoked_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now')),
    reason        TEXT
);

CREATE TABLE revoked_users (
    sub            TEXT PRIMARY KEY,
    revoked_before INTEGER NOT NULL,
    reason         TEXT
);

CREATE TABLE audit_log (
    id            INTEGER PRIMARY KEY,
    actor         TEXT NOT NULL,
    token_id      TEXT,
    action        TEXT NOT NULL,
    class_id      INTEGER,
    booking_id    INTEGER,
    detail_json   TEXT NOT NULL,
    client_ip     TEXT,
    created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ','now'))
);
