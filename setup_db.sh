#!/bin/bash

sqlite3 pinrs.db <<SQL
  CREATE TABLE posts (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL,
    url TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL,
    extended TEXT,
    tags TEXT,
    time TEXT,
    shared TEXT,
    toread TEXT,
    hash TEXT,
    meta TEXT
  );
SQL

sqlite3 pinrs.db <<SQL
  CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username INTEGER NOT NULL UNIQUE,
    token INTEGER NOT NULL
  );
SQL
