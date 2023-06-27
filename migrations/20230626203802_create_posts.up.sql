create table if not exists posts (
    id integer primary key,
    title text not null,
    body text not null,
    account_id int not null references accounts(id),
    updated_at int not null,
    created_at int not null
);