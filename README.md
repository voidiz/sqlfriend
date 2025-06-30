# sqlfriend

LSP-powered SQL line editor and REPL.

## usage

```
$ sqlfriend
Type /help for a list of commands.
sqlfriend> /help
        /add                                - Add a new connection.
        /delete <connection_name>           - Delete a saved connection.
        /help                               - Display a list of available commands.
        /list                               - List all saved connections.
        /set_lsp_server <lsp_server>        - Set the LSP server (Sqls, SqlLs, or PgTools). Should be available in $PATH.
        /use <connection_name>              - Change the active connection.
sqlfriend> /use my_db
Connecting to my_db...
Connected to my_db.
my_db> SELECT * FROM test
 name     | age
----------+--------
 John Doe | 30
my_db>
```

autocompletion is triggered using `<Tab>`.

## roadmap

- [x] readline with (basic) vim support (using [rustyline](https://github.com/kkawakam/rustyline))
- [x] LSP autocompletion
  - [x] using [sqls](https://github.com/sqls-server/sqls)
  - [x] using [sql-language-server](https://github.com/joe-re/sql-language-server)
  - [x] using [postgres-language-server](https://github.com/supabase-community/postgres-language-server)
- [x] LSP diagnostics (very crude)
- [x] execute and print result of SQL queries (postgres, mysql, sqlite)
- [x] save and load database connections
- [ ] meta-commands (such as postgres `\d`)
- [ ] syntax highlighting (treesitter?)

## known issues

If using sqls, auto complete won't list newly created tables until you reconnect to the database (/use).
