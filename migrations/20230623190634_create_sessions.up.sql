create table if not exists sessions (
    id integer not null primary key,
    identifier text not null,
    account_id int not null references accounts(id),
    created_at int not null,
    updated_at int not null
);
create unique index if not exists sessions_identifier on sessions (identifier);
