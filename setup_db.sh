#!/bin/bash

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS posts (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL,
    url TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT,
    notes TEXT,
    unread BOOLEAN,
    time TEXT,
    shared TEXT,
    FOREIGN KEY(user_id) REFERENCES users(id)
  );
SQL

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username INTEGER NOT NULL UNIQUE,
    token TEXT NOT NULL UNIQUE
  );
SQL

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY,
    user_id INTEGER,
    name TEXT NOT NULL,
    UNIQUE(user_id, name),
    FOREIGN KEY(user_id) REFERENCES users(id)
  );
SQL

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS post_tag (
    post_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    UNIQUE(post_id, tag_id),
    FOREIGN KEY(post_id) REFERENCES posts(id) ON DELETE CASCADE,
    FOREIGN KEY(tag_id) REFERENCES tags(id) ON DELETE CASCADE
  );
SQL
