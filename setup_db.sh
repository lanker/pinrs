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
    name INTEGER NOT NULL UNIQUE
  );
SQL

sqlite3 pinrs.db <<SQL
  CREATE TABLE IF NOT EXISTS posttags (
    post_id INTEGER,
    tag_id INTEGER,
    PRIMARY KEY(post_id, tag_id),
    FOREIGN KEY(post_id) REFERENCES posts(id),
    FOREIGN KEY(tag_id) REFERENCES tags(id)
  );
SQL
