create table if not exists accounts (
    id integer not null primary key,
    login_code text not null,
    name text not null,
    updated_at int not null,
    created_at int not null
);
create unique index if not exists accounts_name on accounts(name);
create unique index if not exists accounts_login_code on accounts(login_code);