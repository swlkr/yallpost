PRAGMA defer_foreign_keys = ON;

create table accounts_tmp (
    id integer not null primary key,
    login_code text not null,
    name text not null collate nocase,
    updated_at int not null,
    created_at int not null
);

insert into accounts_tmp (
    id, login_code, name, updated_at, created_at
) select id, login_code, name, updated_at, created_at 
from accounts;

drop index accounts_name;
drop index accounts_login_code;
drop table accounts;

alter table accounts_tmp rename to accounts;
create unique index accounts_name on accounts(name);
create unique index accounts_login_code on accounts(login_code);

PRAGMA defer_foreign_keys = OFF;
