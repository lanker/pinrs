#!/bin/bash

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS posts (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL,
    url TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    extended TEXT,
    time TEXT,
    shared TEXT,
    toread TEXT,
    hash TEXT,
    meta TEXT
  );
SQL

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY,
    username INTEGER NOT NULL UNIQUE,
    token INTEGER NOT NULL
  );
SQL

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY,
    user_id INTEGER,
    name INTEGER NOT NULL,
    UNIQUE(user_id, name)
  );
SQL

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS post_tag (
    post_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    UNIQUE(post_id, tag_id)
  );
SQL
