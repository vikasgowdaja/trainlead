CREATE TABLE users (
    id TEXT PRIMARY KEY,
    email TEXT UNIQUE,
    phone TEXT UNIQUE,
    password_hash TEXT,
    phone_verified INTEGER NOT NULL DEFAULT 0 CHECK(phone_verified IN (0,1)),
    email_verified INTEGER NOT NULL DEFAULT 0 CHECK(email_verified IN (0,1)),
    created_at INTEGER NOT NULL,
    deleted_at INTEGER,
    CHECK(email IS NOT NULL OR phone IS NOT NULL)
);
CREATE TABLE roles (name TEXT PRIMARY KEY, scope TEXT NOT NULL CHECK(scope IN ('global','organization')));
CREATE TABLE permissions (name TEXT PRIMARY KEY);
CREATE TABLE role_permissions (
    role TEXT NOT NULL REFERENCES roles(name),
    permission TEXT NOT NULL REFERENCES permissions(name),
    PRIMARY KEY(role,permission)
);
CREATE TABLE user_roles (
    user_id TEXT NOT NULL REFERENCES users(id),
    role TEXT NOT NULL REFERENCES roles(name),
    PRIMARY KEY(user_id,role)
);
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    family_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id),
    refresh_hash TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    access_expires_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    rotated_at INTEGER,
    revoked_at INTEGER
);
CREATE INDEX sessions_user ON sessions(user_id,revoked_at);
CREATE INDEX sessions_family ON sessions(family_id);
CREATE TABLE otp_challenges (
    id TEXT PRIMARY KEY,
    phone TEXT NOT NULL,
    code_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    consumed_at INTEGER
);
CREATE INDEX otp_phone ON otp_challenges(phone,created_at);
CREATE TABLE oidc_challenges (
    id TEXT PRIMARY KEY,
    nonce_hash TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    consumed_at INTEGER
);
CREATE TABLE external_identities (
    issuer TEXT NOT NULL,
    subject TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id),
    PRIMARY KEY(issuer,subject)
);
CREATE TABLE auth_limits (
    bucket TEXT PRIMARY KEY,
    window_started INTEGER NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    locked_until INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE audit_log (
    id TEXT PRIMARY KEY,
    actor_id TEXT REFERENCES users(id),
    action TEXT NOT NULL,
    object_type TEXT NOT NULL,
    object_id TEXT NOT NULL,
    occurred_at INTEGER NOT NULL,
    outcome TEXT NOT NULL DEFAULT 'success' CHECK(outcome IN ('success','denied'))
);
CREATE INDEX audit_time ON audit_log(occurred_at,id);
CREATE INDEX audit_actor ON audit_log(actor_id,occurred_at);
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_log BEGIN SELECT RAISE(ABORT,'audit log is append-only'); END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_log BEGIN SELECT RAISE(ABORT,'audit log is append-only'); END;
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL);

INSERT INTO roles VALUES ('SuperAdmin','global'),('Admin','global'),('VendorOwner','organization'),('VendorRecruiter','organization'),('Trainer','global'),('Viewer','global');
INSERT INTO permissions VALUES
 ('role:manage'),('audit:read'),('user:read'),('vendor:approve'),('trainer:verify'),
 ('requirement:create'),('requirement:publish'),('requirement:read'),('application:shortlist'),('contact:view');
INSERT INTO role_permissions SELECT 'SuperAdmin',name FROM permissions;
INSERT INTO role_permissions SELECT 'Admin',name FROM permissions WHERE name IN ('audit:read','user:read','vendor:approve','trainer:verify','requirement:read');
INSERT INTO role_permissions VALUES
 ('VendorOwner','requirement:create'),('VendorOwner','requirement:publish'),('VendorOwner','requirement:read'),
 ('VendorOwner','application:shortlist'),('VendorOwner','contact:view'),
 ('VendorRecruiter','requirement:create'),('VendorRecruiter','requirement:read'),
 ('VendorRecruiter','application:shortlist'),('VendorRecruiter','contact:view'),
 ('Trainer','requirement:read'),('Viewer','requirement:read');