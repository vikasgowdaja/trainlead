-- E1: parties, clients, opportunities, postings, chain of custody, reversible merges, event log.
-- All timestamps are UTC unix seconds. IDs are UUID text so the schema ports to Postgres.

CREATE TABLE organizations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('vendor')),
    status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','active','rejected','suspended')),
    created_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE org_members (
    org_id TEXT NOT NULL REFERENCES organizations(id),
    user_id TEXT NOT NULL REFERENCES users(id),
    role TEXT NOT NULL CHECK(role IN ('VendorOwner','VendorRecruiter')),
    PRIMARY KEY(org_id,user_id)
);
CREATE INDEX org_members_user ON org_members(user_id);
-- Phone numbers (digits with country code) that identify a vendor in WhatsApp postings.
CREATE TABLE vendor_contacts (
    phone TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations(id),
    name TEXT
);
CREATE TABLE trainers (
    id TEXT PRIMARY KEY,
    user_id TEXT UNIQUE REFERENCES users(id),
    display_name TEXT NOT NULL,
    city TEXT,
    created_at INTEGER NOT NULL,
    deleted_at INTEGER
);
-- Senders seen in WhatsApp who are not (yet) a known vendor organization.
CREATE TABLE external_contacts (
    id TEXT PRIMARY KEY,
    name TEXT,
    phone TEXT UNIQUE,
    first_seen_at INTEGER NOT NULL
);

CREATE TABLE clients (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('college','university','corporate','institute','other')),
    city TEXT,
    state TEXT,
    contacts TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    deleted_at INTEGER
);
CREATE TABLE client_aliases (
    normalized TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES clients(id)
);

CREATE TABLE opportunities (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK(status IN ('Detected','Verified','Open','Sourcing','Staffed','InDelivery','Completed','Settled','Cancelled','Expired')),
    tech_stack TEXT NOT NULL DEFAULT '[]',
    start_date TEXT,
    end_date TEXT,
    college TEXT,
    city TEXT,
    state TEXT,
    mode TEXT,
    audience TEXT,
    summary TEXT NOT NULL,
    logistics TEXT NOT NULL DEFAULT '{}',
    client_id TEXT REFERENCES clients(id),
    client_match TEXT NOT NULL DEFAULT 'none' CHECK(client_match IN ('none','suggested','confirmed')),
    suggested_client_id TEXT REFERENCES clients(id),
    confidence REAL NOT NULL CHECK(confidence BETWEEN 0 AND 1),
    merged_into TEXT REFERENCES opportunities(id),
    source_cluster_id INTEGER UNIQUE,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX opportunities_status ON opportunities(status,start_date);
CREATE INDEX opportunities_city ON opportunities(city);

CREATE TABLE postings (
    id TEXT PRIMARY KEY,
    opportunity_id TEXT NOT NULL REFERENCES opportunities(id),
    posted_by_org_id TEXT REFERENCES organizations(id),
    external_contact_id TEXT REFERENCES external_contacts(id),
    channel TEXT NOT NULL CHECK(channel IN ('whatsapp','in_app','email','wati')),
    rate_text TEXT,
    rate_amount_inr INTEGER CHECK(rate_amount_inr IS NULL OR rate_amount_inr > 0),
    rate_unit TEXT CHECK(rate_unit IS NULL OR rate_unit IN ('day','hour','session','total')),
    payment_terms TEXT,
    payment_date TEXT,
    logistics TEXT NOT NULL,
    contact_name TEXT,
    contact_phone TEXT,
    raw_text TEXT NOT NULL,
    source_group TEXT,
    sender TEXT,
    posted_at TEXT NOT NULL,
    source_sighting_id INTEGER UNIQUE,
    created_at INTEGER NOT NULL,
    CHECK(posted_by_org_id IS NULL OR external_contact_id IS NULL)
);
CREATE INDEX postings_opportunity ON postings(opportunity_id,posted_at);
CREATE INDEX postings_org ON postings(posted_by_org_id);

-- One hop in the chain of custody. The rate is owned by the from-party (the one paying);
-- it is hidden from the to-party unless explicitly disclosed (e.g. an offer to a trainer).
CREATE TABLE chain_links (
    id TEXT PRIMARY KEY,
    opportunity_id TEXT NOT NULL REFERENCES opportunities(id),
    from_kind TEXT NOT NULL CHECK(from_kind IN ('client','vendor','external','unknown_client')),
    -- For 'unknown_client' this is the opportunity id, keeping UNIQUE effective (NULLs never collide).
    from_id TEXT NOT NULL,
    to_kind TEXT NOT NULL CHECK(to_kind IN ('vendor','external','trainer')),
    to_id TEXT NOT NULL,
    rate_amount_inr INTEGER CHECK(rate_amount_inr IS NULL OR rate_amount_inr > 0),
    rate_unit TEXT CHECK(rate_unit IS NULL OR rate_unit IN ('day','hour','session','total')),
    disclosed_to_recipient INTEGER NOT NULL DEFAULT 0 CHECK(disclosed_to_recipient IN (0,1)),
    inferred INTEGER NOT NULL DEFAULT 0 CHECK(inferred IN (0,1)),
    created_at INTEGER NOT NULL,
    UNIQUE(opportunity_id,from_kind,from_id,to_kind,to_id)
);
CREATE INDEX chain_opportunity ON chain_links(opportunity_id);

CREATE TABLE opportunity_merges (
    id TEXT PRIMARY KEY,
    target_id TEXT NOT NULL REFERENCES opportunities(id),
    source_id TEXT NOT NULL REFERENCES opportunities(id),
    posting_ids TEXT NOT NULL,
    link_ids TEXT NOT NULL,
    merged_by TEXT REFERENCES users(id),
    merged_at INTEGER NOT NULL,
    unmerged_at INTEGER
);

-- Per-object lifecycle event log, append-only like audit_log.
CREATE TABLE object_events (
    id TEXT PRIMARY KEY,
    object_type TEXT NOT NULL,
    object_id TEXT NOT NULL,
    event TEXT NOT NULL,
    from_state TEXT,
    to_state TEXT,
    actor_id TEXT REFERENCES users(id),
    detail TEXT NOT NULL DEFAULT '{}',
    occurred_at INTEGER NOT NULL
);
CREATE INDEX object_events_object ON object_events(object_type,object_id,occurred_at);
CREATE TRIGGER object_events_no_update BEFORE UPDATE ON object_events BEGIN SELECT RAISE(ABORT,'event log is append-only'); END;
CREATE TRIGGER object_events_no_delete BEFORE DELETE ON object_events BEGIN SELECT RAISE(ABORT,'event log is append-only'); END;

INSERT INTO permissions VALUES ('opportunity:merge'),('opportunity:transition'),('client:manage'),('rate:view_all');
INSERT INTO role_permissions VALUES
 ('SuperAdmin','opportunity:merge'),('SuperAdmin','opportunity:transition'),('SuperAdmin','client:manage'),('SuperAdmin','rate:view_all'),
 ('Admin','opportunity:merge'),('Admin','opportunity:transition'),('Admin','client:manage'),('Admin','rate:view_all');
