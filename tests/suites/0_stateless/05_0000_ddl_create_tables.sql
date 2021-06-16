DROP TABLE IF EXISTS t;

CREATE TABLE t(c1 int) ENGINE = Null;
SELECT COUNT(1) from system.tables where name = 't' and database = 'default';

CREATE TABLE IF NOT EXISTS t(c1 int) ENGINE = Null;
CREATE TABLE t(c1 int) ENGINE = Null;

DROP TABLE IF EXISTS t;
